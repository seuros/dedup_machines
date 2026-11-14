# frozen_string_literal: true

require_relative 'test_helper'

# The values are sealed, and sealing stops accidents, not reflection: the
# extension is the authority, and every value it reads back (a ClaimId, an
# Attempt, an Entry) must be exactly its class and valid by the core.
class SealTest < Minitest::Test
  include LedgerHelpers

  VALUES = [ClaimId, Acquired, Completed, Attempt, Entry].freeze

  # A named subclass raises too, but Ruby binds the constant before
  # `inherited` runs: the class leaks, and can be reopened to skip the check.
  def leak(value)
    name = :"Leak#{value.name.split('::').last}"
    return self.class.const_get(name) if self.class.const_defined?(name, false)

    assert_raises(TypeError) { self.class.class_eval("class #{name} < #{value}; end", __FILE__, __LINE__) }
    self.class.const_get(name).tap do |leaked|
      leaked.class_eval { def initialize(**fields) = Data.instance_method(:initialize).bind_call(self, **fields) }
    end
  end

  def test_values_cannot_be_subclassed_or_allocated
    VALUES.each do |value|
      assert_raises(TypeError) { Class.new(value) { def initialize(**) = nil } }
      assert_operator leak(value), :<, value
      assert_raises(NoMethodError) { value.allocate }
    end
    assert_raises(RangeError) { ClaimId.new(1).with(raw: 0) }
  end

  # A forged value never reaches the core and never earns a refusal symbol:
  # a leaked subclass is TypeError, a field `Marshal.load` rewrote RangeError.
  def test_forged_values_are_refused_where_the_extension_reads_them
    ledger = Ledger.new
    id = acquire(ledger, 1, 0, 10)
    fake_id = leak(ClaimId).new(raw: id.raw)
    zero = Marshal.load(Marshal.dump(ClaimId.new(1)).sub("i\x06", "i\x00"))
    assert_equal 0, zero.raw
    assert_unchanged(ledger) do
      [-> { ledger.complete(_1, 0) }, -> { ledger.complete_with(_1, 1, 0) }, -> { ledger.abandon(_1) }].each do |call|
        assert_raises(TypeError) { call.(fake_id) }
        assert_raises(RangeError) { call.(zero) }
      end
      assert_raises(TypeError) { Acquired.new(fake_id) }
      assert_raises(RangeError) { Acquired.new(zero) }
      assert_raises(TypeError) { Entry.new(1, nil, nil, fake_id, 5, nil) }
      assert_raises(RangeError) { Entry.new(1, nil, nil, zero, 5, nil) }

      attempt = leak(Attempt).new(key: -1, fingerprint: nil, partition: nil)
      assert_raises(TypeError) { ledger.admit(attempt, 0, 10) }
      negative = Marshal.load(Marshal.dump(Attempt.new(2, nil, nil)).sub("i\x07", "i\xFA".b))
      assert_equal(-1, negative.key)
      assert_raises(RangeError) { ledger.admit(negative, 0, 10) }
    end

    entry = leak(Entry).new(key: 1, fingerprint: nil, partition: nil, claim: zero, expires: 5, value: nil)
    assert_raises(TypeError) { Ledger.resumed([entry], 0, 1, 0) }
    saved = Entry.new(1, nil, nil, ClaimId.new(1), 5, nil)
    unissued = Marshal.load(Marshal.dump(saved).gsub("i\x06", "i\x00"))
    assert_equal 0, unissued.claim.raw
    assert_raises(RangeError) { Ledger.resumed([unissued], 0, 1, 0) }
  end

  # A singleton class opened on a genuine value does not change what it is,
  # a clone of one included, wherever the extension reads it back.
  def test_an_opened_singleton_class_is_still_the_gems_value
    ledger = Ledger.new
    id = acquire(ledger, 1, 0, 10).tap(&:singleton_class)
    assert_equal Acquired.new(id), Acquired.new(id.clone)
    assert_equal :completed, ledger.complete(id.clone, 1)
    assert_instance_of Acquired, ledger.admit(Attempt.new(2, nil, nil).tap(&:singleton_class), 1, 10)
    entries = ledger.entries.map { _1.clone.tap(&:singleton_class) }
    assert_equal ledger.entries, Ledger.resumed(entries, 0, ledger.issued, 1).entries
  end

  # The extension reads every value's fields from the struct, never through
  # its readers: a redefined reader is ignored, and cannot re-enter.
  def test_a_redefined_reader_is_ignored
    ledger = Ledger.new
    id = acquire(ledger, 1, 0, 10)
    attempt = Attempt.new(2, 7, nil)
    readers = { ClaimId => %i[raw], Attempt => %i[key fingerprint], Entry => %i[key claim expires] }
    originals = readers.flat_map { |klass, names| names.map { [klass, it, klass.instance_method(it)] } }
    begin
      originals.each { |klass, name, _| klass.define_method(name) { raise "#{name} reader ran" } }
      assert_equal :completed, ledger.complete(id, 1)
      assert_equal :in_progress, ledger.admit(attempt, 1, 10).then { ledger.admit(attempt, 1, 10) }
      saved = ledger.entries
      restored = Ledger.resumed(saved, 0, ledger.issued, 1)
    ensure
      originals.each { |klass, name, reader| klass.define_method(name, reader) }
    end
    assert_equal ledger.entries, restored.entries
  end

  # The classes the extension resolved at init are pinned: after a
  # compaction that moves what it can, every path that builds or reads a
  # value still finds them.
  def test_resolved_classes_survive_compaction
    GC.verify_compaction_references(expand_heap: true, toward: :empty) if GC.respond_to?(:verify_compaction_references)
    GC.compact if GC.respond_to?(:compact)
    ledger = Ledger.new
    id = acquire(ledger, 1, 0, 10)
    assert_equal :completed, ledger.complete_with(id, 9, 0)
    assert_equal Completed.new(9), ledger.claim(1, 1, 10)
    assert_instance_of Acquired, ledger.admit(Attempt.new(2, nil, 3), 1, 10)
    entries = ledger.entries
    assert_equal 2, entries.size
    assert_equal entries, Ledger.resumed(entries, 0, ledger.issued, 1).entries
  end
end
