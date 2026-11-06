# frozen_string_literal: true
require 'minitest/autorun'
require 'dedup_machines'

class LedgerTest < Minitest::Test
  def test_exact_completion_and_expiry
    ledger = DedupMachines::Ledger.new
    claim = ledger.claim(42, 0, 10)
    assert_instance_of DedupMachines::Claim, claim
    assert_equal :in_progress, ledger.claim(42, 1, 10)
    assert ledger.complete(claim, 2)
    refute ledger.complete(claim, 2)
    refute ledger.abandon(claim)
    assert_equal :completed, ledger.claim(42, 9, 10)
    fresh = ledger.claim(42, 10, 10)
    assert_instance_of DedupMachines::Claim, fresh
    refute ledger.complete(claim, 10)
    assert ledger.abandon(fresh)
  end

  def test_opaque_owner_lifetime_and_gc
    a = DedupMachines::Ledger.new
    b = DedupMachines::Ledger.new
    claim = a.claim(1, 0, 10)
    b.claim(1, 0, 10)
    refute b.complete(claim, 0)
    refute b.abandon(claim)
    assert_raises(TypeError) { DedupMachines::Claim.allocate }
    assert_raises(TypeError) { claim.dup }
    a = nil
    GC.start
    refute DedupMachines::Ledger.new.abandon(claim)
  end

  def test_capacity_and_stale_reuse
    ledger = DedupMachines::Ledger.new
    assert_equal 64, ledger.capacity
    claims = 64.times.map { |i| ledger.claim(i, 0, 100) }
    assert_equal :full, ledger.claim(65, 0, 100)
    assert ledger.abandon(claims.first)
    assert_instance_of DedupMachines::Claim, ledger.claim(65, 0, 100)
    refute ledger.abandon(claims.first)
  end

  def test_strict_inputs_before_mutation_and_time_policy
    ledger = DedupMachines::Ledger.new
    bad = [1.0, Float::INFINITY, Float::NAN, nil, '1', Object.new]
    bad.each do |value|
      assert_raises(TypeError) { ledger.claim(value, 0, 10) }
      assert_raises(TypeError) { ledger.claim(1, value, 10) }
      assert_raises(TypeError) { ledger.claim(1, 0, value) }
    end
    [-1, 2**64].each { |n| assert_raises(RangeError) { ledger.claim(n, 0, 10) } }
    assert_equal :rejected, ledger.claim(1, 0, 0)
    claim = ledger.claim(1, 10, 10)
    assert_raises(TypeError) { ledger.complete(claim, 10.5) }
    refute ledger.complete(claim, 9)
    assert ledger.complete(claim, 10)
    assert_equal :rejected, ledger.claim(2, 9, 10)
  end

  def test_freeze_guards
    ledger = DedupMachines::Ledger.new
    claim = ledger.claim(1, 0, 10)
    ledger.freeze
    assert_raises(FrozenError) { ledger.claim(2, 0, 10) }
    assert_raises(FrozenError) { ledger.complete(claim, 0) }
    assert_raises(FrozenError) { ledger.abandon(claim) }
    assert_equal 64, ledger.capacity
  end

  def test_gc_stress_and_duration_upper_bound
    previous = GC.stress
    GC.stress = true
    ledger = DedupMachines::Ledger.new
    claim = ledger.claim(2**64 - 1, 2**64 - 1, 2**64 - 1)
    assert ledger.complete(claim, 2**64 - 1)
    assert_equal :completed, ledger.claim(2**64 - 1, 2**64 - 1, 1)
    GC.compact
    assert_equal :completed, ledger.claim(2**64 - 1, 2**64 - 1, 1)
  ensure
    GC.stress = previous
  end
end
