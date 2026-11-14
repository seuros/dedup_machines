# frozen_string_literal: true
require_relative 'test_helper'

# A refusal is a symbol, never an exception, and it changes nothing.
class RefusalsTest < Minitest::Test
  include LedgerHelpers

  # Key 9 expired at 1, key 1 pending under fingerprint 0xA in partition 4,
  # key 2 completed with 7; a partition cap of 1 and the clock at 2.
  def busy
    ledger = Ledger.new.with_partition_cap(1)
    expired = acquire(ledger, 9, 0, 1)
    pending = ledger.admit(Attempt.new(1, 0xA, 4), 0, 20).id
    done = acquire(ledger, 2, 0, 20)
    assert_equal :completed, ledger.complete_with(done, 7, 2)
    [ledger, expired, pending, done]
  end

  def test_refused_claims_change_nothing
    ledger, = busy
    assert_unchanged(ledger) do
      assert_equal :time_regression, ledger.claim(5, 1, 5)
      assert_equal :invalid_duration, ledger.claim(5, 3, 0)
      assert_equal :time_regression, ledger.claim(1, 1, 0)
      assert_equal :conflict, ledger.claim(1, 3, 5)
      assert_equal :conflict, ledger.claim_with(1, 0xB, 3, 5)
      assert_equal :conflict, ledger.claim_with(2, 0xB, 3, 5)
      assert_equal :partition_full, ledger.admit(Attempt.new(5, nil, 4), 3, 5)
    end
    assert_equal 3, ledger.entries.size
  end

  def test_honoured_duplicate_answers_move_only_the_clock
    ledger, = busy
    before = state(ledger)
    assert_equal :in_progress, ledger.claim_with(1, 0xA, 3, 5)
    assert_equal Completed.new(7), ledger.claim(2, 4, 5)
    assert_equal 4, ledger.observed
    assert_equal before[0..2], state(ledger)[0..2]
  end

  def test_refused_and_repeated_completions_change_nothing
    ledger, expired, pending, done = busy
    never = ClaimId.new(ledger.issued + 1)
    assert_unchanged(ledger) do
      assert_equal :time_regression, ledger.complete(pending, 1)
      assert_equal :stale, ledger.complete(never, 1)
      assert_equal :stale, ledger.complete(expired, 3)
      assert_equal :stale, ledger.complete(pending, 20)
      assert_equal :mismatch, ledger.complete_with(done, 8, 3)
      assert_equal :mismatch, ledger.complete(done, 3)
      assert_equal :already_completed, ledger.complete_with(done, 7, 19)
    end
  end

  def test_refused_abandons_change_nothing
    ledger, expired, pending, done = busy
    assert_unchanged(ledger) do
      assert_equal :stale, ledger.abandon(ClaimId.new(ledger.issued + 1))
      assert_equal :stale, ledger.abandon(expired)
      assert_equal :concluded, ledger.abandon(done)
    end
    assert_equal :abandoned, ledger.abandon(pending)
    assert_unchanged(ledger) { assert_equal :stale, ledger.abandon(pending) }
  end

  # :stale for an expired claim answers for the instant supplied. Nothing was
  # recorded, so the same completion at an earlier instant is honoured.
  def test_a_refused_expiry_is_not_remembered
    ledger = Ledger.new
    id = acquire(ledger, 1, 0, 10)
    assert_unchanged(ledger) { assert_equal :stale, ledger.complete_with(id, 7, 10) }
    assert_equal :completed, ledger.complete_with(id, 7, 5)
    assert_equal 5, ledger.observed
    assert_equal Completed.new(7), ledger.claim(1, 6, 10)
  end

  # An honoured claim of the key is recorded, and ends the old id for good.
  def test_an_honoured_claim_ends_the_expired_one
    ledger = Ledger.new
    old = acquire(ledger, 1, 0, 10)
    assert_unchanged(ledger) { assert_equal :stale, ledger.complete(old, 10) }
    fresh = acquire(ledger, 1, 10, 10)
    assert_unchanged(ledger) do
      assert_equal :time_regression, ledger.complete(old, 5)
      assert_equal :stale, ledger.complete(old, 10)
      assert_equal :stale, ledger.abandon(old)
    end
    assert_equal :completed, ledger.complete(fresh, 10)
  end

  def test_exhaustion_and_fullness_are_distinct
    ledger = Ledger.resumed([], 3, 2**48 - 2, 0)
    last = acquire(ledger, 9, 0, 1)
    assert_equal [3, 2**48 - 1], [last.scope, ledger.issued]
    assert_unchanged(ledger) { assert_equal :exhausted, ledger.claim(1, 2, 9) }
  end

  def test_every_refusal_symbol_is_reachable_but_overflow
    # `now + ttl` cannot overflow a Duration from Integer nanoseconds below
    # 2**64: :overflow is the one refusal Ruby cannot reach.
    ledger = Ledger.new
    assert_instance_of Acquired, ledger.claim(1, 2**64 - 1, 2**64 - 1)
  end
end
