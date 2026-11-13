# frozen_string_literal: true
require 'minitest/autorun'
require 'dedup_machines'

module LedgerHelpers
  include DedupMachines

  # Everything a ledger holds: a refused call must leave it unchanged.
  def state(ledger)
    [ledger.entries, ledger.scope, ledger.issued, ledger.observed, ledger.partition_cap]
  end

  def acquire(ledger, key, now, ttl)
    answer = ledger.claim(key, now, ttl)
    assert_instance_of Acquired, answer
    answer.id
  end

  def assert_unchanged(ledger)
    before = state(ledger)
    yield
    assert_equal before, state(ledger)
  end
end
