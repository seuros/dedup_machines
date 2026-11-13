# frozen_string_literal: true
require_relative 'test_helper'

# Claim ids carry the scope of the ledger that issued them, and a restart
# resumes the sequence instead of rewinding it.
class ScopeTest < Minitest::Test
  include LedgerHelpers

  def settled(scope)
    ledger = Ledger.with_scope(scope)
    pending = acquire(ledger, 1, 0, 20)
    done = acquire(ledger, 2, 0, 20)
    assert_equal :completed, ledger.complete(done, 0)
    [ledger, pending, done]
  end

  def test_ids_carry_the_scope
    ledger, pending, done = settled(7)
    assert_equal [7, 2], [ledger.scope, ledger.issued]
    assert_equal [(7 << 48) | 1, (7 << 48) | 2], [pending.raw, done.raw]
    assert_equal [7, 7], [pending.scope, done.scope]
    assert_equal 0, Ledger.new.scope
    assert_equal 1, acquire(Ledger.new, 1, 0, 1).raw
    refute_respond_to Ledger, :__scope_of
  end

  def test_scope_is_validated
    assert_raises(TypeError) { Ledger.with_scope(1.0) }
    assert_raises(TypeError) { Ledger.with_scope(nil) }
    assert_raises(RangeError) { Ledger.with_scope(-1) }
    assert_raises(RangeError) { Ledger.with_scope(65_536) }
    assert_equal 65_535, Ledger.with_scope(65_535).scope
  end

  def test_a_claim_from_another_ledger_settles_nothing
    near, near_pending, near_done = settled(1)
    far, far_pending, far_done = settled(2)
    [[near, far_pending, far_done], [far, near_pending, near_done]].each do |ledger, pending, done|
      assert_unchanged(ledger) do
        assert_equal :stale, ledger.complete(pending, 1)
        assert_equal :stale, ledger.abandon(pending)
        assert_equal :stale, ledger.complete(done, 1)
        assert_equal :stale, ledger.abandon(done)
      end
    end
    assert_equal :completed, near.complete(near_pending, 1)
    assert_equal :abandoned, far.abandon(far_pending)
  end

  def test_a_claim_id_crosses_a_queue_as_an_integer
    ledger, pending, done = settled(5)
    pending, done = [pending.raw, done.raw].map { |raw| ClaimId.new(raw) }
    assert_equal :already_completed, ledger.complete(done, 1)
    assert_equal :abandoned, ledger.abandon(pending)
    [0, (5 << 48) | 3, (4 << 48) | 2, 2**64 - 1].each do |raw|
      assert_equal :stale, ledger.abandon(ClaimId.new(raw))
    end
  end

  def test_a_restart_resumes_the_sequence
    ledger, pending, done = settled(3)
    gone = acquire(ledger, 3, 1, 20)
    assert_equal :abandoned, ledger.abandon(gone)
    saved = [ledger.entries, ledger.scope, ledger.issued, ledger.observed]

    restored = Ledger.resumed(*saved)
    assert_equal state(ledger), state(restored)
    assert_equal :time_regression, restored.claim(0, 0, 5)
    assert_equal :already_completed, restored.complete(done, 1)
    assert_equal :stale, restored.abandon(gone)
    assert_equal (3 << 48) | 4, acquire(restored, 3, 1, 20).raw
    assert_equal :abandoned, restored.abandon(pending)
  end

  def test_saved_state_of_another_scope_is_refused
    ledger, = settled(3)
    entries, scope, issued, observed = ledger.entries, ledger.scope, ledger.issued, ledger.observed
    twin_key = [entries.first.with(key: entries.last.key), entries.last]
    twin_claim = [entries.first.with(claim: entries.last.claim), entries.last]
    same_twice = [entries.first, entries.first]
    unsound = [
      [entries, 4, issued, observed], [entries, scope, 1, observed], [entries, scope, 0, observed],
      [twin_key, scope, issued, observed], [twin_claim, scope, issued, observed],
      [same_twice, scope, issued, observed]
    ]
    unsound.each do |saved|
      error = assert_raises(RangeError) { Ledger.resumed(*saved) }
      assert_match(/saved state/, error.message)
    end
    assert_instance_of Ledger, Ledger.resumed([], 4, issued, observed)
  end

  def test_resumed_validates_its_input
    entry = Entry.new(1, nil, nil, ClaimId.new(1), 10, nil)
    assert_instance_of Ledger, Ledger.resumed([entry], 0, 1, 0)
    assert_raises(TypeError) { Ledger.resumed(nil, 0, 1, 0) }
    assert_raises(TypeError) { Ledger.resumed([1], 0, 1, 0) }
    assert_raises(TypeError) { Ledger.resumed([entry.with(claim: 1)], 0, 1, 0) }
    assert_raises(TypeError) { Ledger.resumed([entry.with(expires: 1.5)], 0, 1, 0) }
    assert_raises(TypeError) { Ledger.resumed([entry.with(value: 'x')], 0, 1, 0) }
    assert_raises(TypeError) { Ledger.resumed([entry], 0, 1.0, 0) }
    assert_raises(RangeError) { Ledger.resumed([entry.with(expires: -1)], 0, 1, 0) }
    assert_raises(RangeError) { Ledger.resumed([entry.with(expires: 2**100)], 0, 1, 0) }
    assert_raises(RangeError) { Ledger.resumed([entry], 0, 2**48, 0) }
    assert_raises(RangeError) { Ledger.resumed([entry], 65_536, 1, 0) }
    many = 65.times.map { |i| Entry.new(i, nil, nil, ClaimId.new(i + 1), 10, nil) }
    assert_raises(RangeError) { Ledger.resumed(many, 0, 65, 0) }
    assert_equal 64, Ledger.resumed(many.first(64), 0, 64, 0).entries.size
    error = assert_raises(RangeError) { Ledger.resumed(many, 0, 65, 0) }
    assert_match(/at most 64/, error.message)
  end
end
