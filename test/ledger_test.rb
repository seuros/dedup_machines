# frozen_string_literal: true
require_relative 'test_helper'

class LedgerTest < Minitest::Test
  include LedgerHelpers

  def test_exact_completion_and_expiry
    ledger = Ledger.new
    id = acquire(ledger, 42, 0, 10)
    assert_equal ClaimId.new(1), id
    assert_equal :in_progress, ledger.claim(42, 1, 10)
    assert_equal :completed, ledger.complete(id, 2)
    assert_equal :already_completed, ledger.complete(id, 2)
    assert_equal :concluded, ledger.abandon(id)
    assert_equal Completed.new(0), ledger.claim(42, 9, 10)
    fresh = acquire(ledger, 42, 10, 10)
    assert_equal 2, fresh.raw
    assert_equal :stale, ledger.complete(id, 10)
    assert_equal :abandoned, ledger.abandon(fresh)
    assert_equal :stale, ledger.abandon(fresh)
  end

  def test_completion_values_reach_duplicates
    ledger = Ledger.new
    id = acquire(ledger, 7, 0, 10)
    assert_equal :completed, ledger.complete_with(id, 42, 1)
    assert_equal Completed.new(42), ledger.claim(7, 2, 10)
    assert_unchanged(ledger) do
      assert_equal :already_completed, ledger.complete_with(id, 42, 3)
      assert_equal :mismatch, ledger.complete_with(id, 43, 3)
      assert_equal :mismatch, ledger.complete(id, 3)
    end
    assert_equal 2**64 - 1, completed_value(ledger, 2**64 - 1)
  end

  def test_fingerprints_and_attempts
    ledger = Ledger.new.with_partition_cap(1)
    id = ledger.claim_with(7, 0xA, 0, 10).id
    assert_equal :conflict, ledger.claim_with(7, 0xB, 1, 10)
    assert_equal :conflict, ledger.claim(7, 1, 10)
    assert_equal :in_progress, ledger.claim_with(7, 0xA, 1, 10)
    assert_equal :in_progress, ledger.admit(Attempt.new(7, 0xA, nil), 1, 10)
    assert_equal :completed, ledger.complete(id, 2)
    assert_equal Completed.new(0), ledger.admit(Attempt.new(7, 0xA, 3), 2, 10)

    assert_instance_of Acquired, ledger.admit(Attempt.new(1, nil, 9), 2, 10)
    assert_equal :partition_full, ledger.admit(Attempt.new(2, nil, 9), 2, 10)
    assert_instance_of Acquired, ledger.admit(Attempt.new(2, nil, 8), 2, 10)
    assert_instance_of Acquired, ledger.admit(Attempt.new(3, nil, nil), 2, 10)
  end

  def test_partition_cap_returns_a_capped_copy
    plain = Ledger.with_scope(3)
    acquire(plain, 1, 0, 10)
    capped = plain.with_partition_cap(2)
    assert_nil plain.partition_cap
    assert_equal 2, capped.partition_cap
    assert_equal state(plain)[0..3], state(capped)[0..3]
    assert_instance_of Acquired, capped.claim(2, 0, 10)
    assert_equal 1, plain.entries.size
  end

  def test_capacity_and_lazy_expiry
    ledger = Ledger.new
    assert_equal 64, ledger.capacity
    assert_equal 64, DedupMachines::CAPACITY
    ids = 64.times.map { |i| acquire(ledger, i, 0, i.zero? ? 5 : 100) }
    assert_unchanged(ledger) { assert_equal :full, ledger.claim(65, 4, 100) }
    # Key 0 expired at 5: read as absent, removed by the claim that takes its slot.
    assert_equal :in_progress, ledger.claim(1, 5, 100)
    assert_equal 64, ledger.entries.size
    assert_equal :stale, ledger.abandon(ids.first)
    assert_instance_of Acquired, ledger.claim(65, 5, 100)
    assert_equal 64, ledger.entries.size
    assert_equal :full, ledger.claim(0, 5, 100)
    assert_equal :abandoned, ledger.abandon(ids[1])
    assert_instance_of Acquired, ledger.claim(0, 5, 100)
  end

  def test_expire_where_drains_by_key
    ledger = Ledger.new
    dead = acquire(ledger, 10, 0, 100)
    done = acquire(ledger, 11, 0, 100)
    alive = acquire(ledger, 20, 0, 100)
    acquire(ledger, 12, 0, 1)
    assert_equal :completed, ledger.complete(done, 1)
    seen = []
    dropped = ledger.expire_where do |key|
      seen << key
      key < 20
    end
    assert_equal 2, dropped
    # Key 12 expired at the observed clock: not offered, not counted, removed.
    assert_equal [10, 11, 20], seen.sort
    assert_equal [20], ledger.entries.map(&:key)
    assert_equal 0, ledger.expire_where { true if it == 10 }
    assert_equal :stale, ledger.abandon(dead)
    assert_equal :completed, ledger.complete(alive, 1)
    assert_raises(ArgumentError) { ledger.expire_where }
    assert_unchanged(ledger) { assert_raises(RuntimeError) { ledger.expire_where { raise 'no' } } }
  end

  def test_data_values_mirror_the_core
    assert_equal %i[raw], ClaimId.members
    assert_equal %i[id], Acquired.members
    assert_equal %i[value], Completed.members
    assert_equal %i[key fingerprint partition], Attempt.members
    assert_equal %i[key fingerprint partition claim expires value], Entry.members
    ledger = Ledger.new
    id = ledger.admit(Attempt.new(5, 6, 7), 1, 2**64 - 1).id
    entry = Entry.new(5, 6, 7, id, 2**64, nil)
    assert_equal [entry], ledger.entries
    assert_predicate ledger.entries, :frozen?
    assert_predicate entry, :pending?
    ledger.complete_with(id, 9, 1)
    refute_predicate ledger.entries.first, :pending?
    assert_equal 9, ledger.entries.first.value
  end

  def test_strict_inputs_raise_before_any_change
    ledger = Ledger.new
    id = acquire(ledger, 1, 10, 10)
    bad = [1.0, Float::NAN, nil, '1', Object.new]
    assert_unchanged(ledger) do
      bad.each do |value|
        assert_raises(TypeError) { ledger.claim(value, 10, 10) }
        assert_raises(TypeError) { ledger.claim(2, value, 10) }
        assert_raises(TypeError) { ledger.claim(2, 10, value) }
        assert_raises(TypeError) { ledger.claim_with(2, value, 10, 10) }
        assert_raises(TypeError) { ledger.complete(id, value) }
        assert_raises(TypeError) { ledger.complete_with(id, value, 10) }
        assert_raises(TypeError) { ledger.complete(value, 10) }
        assert_raises(TypeError) { ledger.abandon(value) }
        assert_raises(TypeError) { ledger.admit(value, 10, 10) }
        assert_raises(TypeError) { ledger.abandon(ClaimId.new(value)) }
        assert_raises(TypeError) { ledger.with_partition_cap(value) }
      end
      assert_raises(TypeError) { ledger.admit(Attempt.new(2, 1.5, nil), 10, 10) }
      assert_raises(TypeError) { ledger.abandon(id.raw) }
      [-1, 2**64].each do |n|
        assert_raises(RangeError) { ledger.claim(n, 10, 10) }
        assert_raises(RangeError) { ledger.claim(2, n, 10) }
        assert_raises(RangeError) { ledger.complete_with(id, n, 10) }
        assert_raises(RangeError) { ledger.abandon(ClaimId.new(n)) }
      end
    end
  end

  def test_freeze_guards
    ledger = Ledger.new
    id = acquire(ledger, 1, 0, 10)
    ledger.freeze
    assert_raises(FrozenError) { ledger.claim(2, 0, 10) }
    assert_raises(FrozenError) { ledger.claim_with(2, 1, 0, 10) }
    assert_raises(FrozenError) { ledger.admit(Attempt.new(2, nil, nil), 0, 10) }
    assert_raises(FrozenError) { ledger.complete(id, 0) }
    assert_raises(FrozenError) { ledger.complete_with(id, 1, 0) }
    assert_raises(FrozenError) { ledger.abandon(id) }
    assert_raises(FrozenError) { ledger.expire_where { true } }
    assert_equal 64, ledger.capacity
    assert_equal 1, ledger.entries.size
    refute_predicate ledger.with_partition_cap(1), :frozen?
  end

  # Frozen is checked before any argument is read: a frozen ledger handed a
  # malformed argument raises FrozenError, as Ruby's own core methods do.
  def test_frozen_is_checked_before_the_arguments
    ledger = Ledger.new.freeze
    assert_raises(FrozenError) { ledger.claim(1.5, 0, 10) }
    assert_raises(FrozenError) { ledger.claim_with(1, nil, 0, 10) }
    assert_raises(FrozenError) { ledger.admit(:attempt, 0, 10) }
    assert_raises(FrozenError) { ledger.complete(nil, 0) }
    assert_raises(FrozenError) { ledger.complete_with(1, -1, 0) }
    assert_raises(FrozenError) { ledger.abandon('id') }
  end

  # Entries are read natively from exactly an Array: a subclass, whose
  # iteration it could redefine, is a TypeError.
  def test_resumed_entries_must_be_exactly_an_array
    ledger = Ledger.new
    acquire(ledger, 1, 0, 10)
    subclass = Class.new(Array)
    assert_raises(TypeError) { Ledger.resumed(subclass.new(ledger.entries), 0, ledger.issued, 0) }
    assert_equal ledger.entries, Ledger.resumed(ledger.entries, 0, ledger.issued, 0).entries
  end

  def test_gc_stress_and_duration_upper_bound
    previous = GC.stress
    GC.stress = true
    ledger = Ledger.new
    top = 2**64 - 1
    id = acquire(ledger, top, top, top)
    assert_equal :completed, ledger.complete_with(id, top, top)
    assert_equal Completed.new(top), ledger.claim(top, top, 1)
    GC.compact
    assert_equal Completed.new(top), ledger.claim(top, top, 1)
    assert_equal top, ledger.observed
    assert_equal 2 * top, ledger.entries.first.expires
  ensure
    GC.stress = previous
  end

  private

  def completed_value(ledger, value)
    id = acquire(ledger, 99, 3, 10)
    ledger.complete_with(id, value, 3)
    ledger.claim(99, 3, 10).value
  end
end
