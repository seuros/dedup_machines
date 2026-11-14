# frozen_string_literal: true
require_relative 'test_helper'

# Every Data value validates at construction through the core: a value built
# by hand is held to the rule a returned one is.
class ValuesTest < Minitest::Test
  include LedgerHelpers

  BAD_TYPES = [1.0, nil, '1', :one, Object.new].freeze
  OUT_OF_RANGE = [-1, 2**64].freeze

  def test_a_claim_id_of_sequence_zero_does_not_exist
    [0, 7 << 48, 65_535 << 48].each do |raw|
      assert_raises(RangeError) { ClaimId.new(raw) }
    end
    [1, (7 << 48) | 1, 2**64 - 1].each { |raw| assert_equal raw, ClaimId.new(raw).raw }
    assert_equal 7, ClaimId.new((7 << 48) | 1).scope
    BAD_TYPES.each { |bad| assert_raises(TypeError) { ClaimId.new(bad) } }
    OUT_OF_RANGE.each { |bad| assert_raises(RangeError) { ClaimId.new(bad) } }
  end

  def test_acquired_holds_a_claim_id
    id = ClaimId.new(1)
    assert_equal id, Acquired.new(id).id
    (BAD_TYPES + [1]).each { |bad| assert_raises(TypeError) { Acquired.new(bad) } }
  end

  def test_completed_holds_an_unsigned_integer
    assert_equal 0, Completed.new(0).value
    BAD_TYPES.each { |bad| assert_raises(TypeError) { Completed.new(bad) } }
    OUT_OF_RANGE.each { |bad| assert_raises(RangeError) { Completed.new(bad) } }
  end

  def test_attempt_holds_a_key_and_optional_integers
    assert_equal [1, nil, nil], Attempt.new(1, nil, nil).deconstruct
    assert_equal [1, 2, 3], Attempt.new(key: 1, fingerprint: 2, partition: 3).deconstruct
    BAD_TYPES.each { |bad| assert_raises(TypeError) { Attempt.new(bad, nil, nil) } }
    (BAD_TYPES - [nil]).each do |bad|
      assert_raises(TypeError) { Attempt.new(1, bad, nil) }
      assert_raises(TypeError) { Attempt.new(1, nil, bad) }
    end
    OUT_OF_RANGE.each do |bad|
      assert_raises(RangeError) { Attempt.new(bad, nil, nil) }
      assert_raises(RangeError) { Attempt.new(1, bad, nil) }
      assert_raises(RangeError) { Attempt.new(1, nil, bad) }
    end
  end

  def test_entry_holds_what_entry_new_takes
    id = ClaimId.new(1)
    assert_equal [1, nil, nil, id, 2**64, nil], Entry.new(1, nil, nil, id, 2**64, nil).deconstruct
    assert_raises(TypeError) { Entry.new(1, nil, nil, 1, 5, nil) }
    assert_raises(TypeError) { Entry.new(1, nil, nil, id, 5.0, nil) }
    assert_raises(TypeError) { Entry.new(1, nil, nil, id, nil, nil) }
    assert_raises(TypeError) { Entry.new(1, nil, nil, id, 5, '9') }
    assert_raises(TypeError) { Entry.new(nil, nil, nil, id, 5, nil) }
    assert_raises(RangeError) { Entry.new(-1, nil, nil, id, 5, nil) }
    assert_raises(RangeError) { Entry.new(1, -1, nil, id, 5, nil) }
    assert_raises(RangeError) { Entry.new(1, nil, nil, id, -5, nil) }
    assert_raises(RangeError) { Entry.new(1, nil, nil, id, 2**128, nil) }
    assert_raises(RangeError) { Entry.new(1, nil, nil, id, 5, 2**64) }
  end

  # The block runs before anything is dropped and with the ledger free. A
  # block that abandons a key and claims it again shows it the old claim
  # only: the new claim is not dropped on the old one's verdict.
  def test_expire_where_drops_only_the_records_the_block_was_shown
    ledger = Ledger.new
    old = acquire(ledger, 1, 0, 100)
    acquire(ledger, 2, 0, 100)
    fresh = nil
    dropped = ledger.expire_where do |key|
      if key == 1
        assert_equal :abandoned, ledger.abandon(old)
        fresh = acquire(ledger, 1, 0, 100)
      end
      true
    end
    assert_equal 1, dropped
    assert_equal [[1, fresh]], ledger.entries.map { [it.key, it.claim] }
  end

  def test_expire_where_drops_nothing_when_the_block_freezes_the_ledger
    ledger = Ledger.new
    acquire(ledger, 1, 0, 100)
    before = state(ledger)
    assert_raises(FrozenError) { ledger.expire_where { ledger.freeze } }
    assert_equal before, state(ledger)
  end
end
