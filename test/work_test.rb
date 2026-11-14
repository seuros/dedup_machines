# frozen_string_literal: true

require_relative 'test_helper'

# The ledger never loses work. A claim answers Ruby objects (an Acquired
# holding a ClaimId, or a Completed), so the core decides on a copy, the
# answer is built, and the copy is kept only if the ledger is still unfrozen
# and unchanged. A `new` that raises, calls back in, or freezes the ledger
# applies nothing: no claim is taken whose id the caller never saw.
class WorkTest < Minitest::Test
  include LedgerHelpers

  # Each claim that builds values: the ledger it runs on, the call, and every
  # class whose `new` it runs.
  CALLS = {
    claim: [->(_) {}, ->(ledger) { ledger.claim(1, 5, 10) }, [ClaimId, Acquired]],
    claim_with: [->(_) {}, ->(ledger) { ledger.claim_with(1, 7, 5, 10) }, [ClaimId, Acquired]],
    admit: [->(_) {}, ->(ledger) { ledger.admit(Attempt.new(1, nil, 3), 5, 10) }, [ClaimId, Acquired]],
    completed: [
      ->(ledger) { ledger.complete_with(ledger.claim(1, 0, 100).id, 9, 0) },
      ->(ledger) { ledger.claim(1, 5, 100) },
      [Completed]
    ]
  }.freeze

  # Runs the block with `klass.new` replaced by `body`, the way reopening the
  # class could redefine it; the real `new` is `klass.__real_new`.
  def replacing_new(klass, body)
    klass.singleton_class.alias_method(:__real_new, :new)
    klass.define_singleton_method(:new, &body)
    yield
  ensure
    klass.singleton_class.alias_method(:new, :__real_new)
    klass.singleton_class.remove_method(:__real_new)
  end

  # The ledger and a twin given the same history.
  def pair(setup) = Array.new(2) { Ledger.new.tap(&setup) }

  def each_case
    CALLS.each do |name, (setup, call, built)|
      built.each { |klass| yield name.to_s, *pair(setup), call, klass }
    end
  end

  def test_a_new_that_raises_applies_nothing
    each_case do |name, ledger, twin, call, klass|
      replacing_new(klass, ->(*) { raise IOError }) { assert_raises(IOError, name) { call.(ledger) } }
      assert_equal state(twin), state(ledger), name
      # Not even the clock moved, and no id was spent.
      assert_equal call.(twin), call.(ledger), name
    end
  end

  def test_a_new_that_calls_back_in_applies_nothing
    each_case do |name, ledger, twin, call, klass|
      called = false
      reenter = lambda do |*args|
        # The inner claim builds values too: call back in once.
        unless called
          called = true
          ledger.claim(2, 0, 10)
        end
        klass.__real_new(*args)
      end
      replacing_new(klass, reenter) { assert_raises(RuntimeError, name) { call.(ledger) } }
      twin.claim(2, 0, 10)
      assert_equal state(twin), state(ledger), name
      assert_equal call.(twin), call.(ledger), name
    end
  end

  def test_a_new_that_freezes_the_ledger_applies_nothing
    each_case do |name, ledger, twin, call, klass|
      freeze = lambda do |*args|
        ledger.freeze
        klass.__real_new(*args)
      end
      replacing_new(klass, freeze) { assert_raises(FrozenError, name) { call.(ledger) } }
      assert_equal state(twin), state(ledger), name
    end
  end

  # `entries` builds values too. It holds no borrow while `new` runs, so a
  # `new` that calls back in is an ordinary call, never a `fatal`.
  def test_entries_holds_no_borrow_while_values_are_built
    ledger = Ledger.new
    acquire(ledger, 1, 0, 10)
    reenter = lambda do |*args|
      ledger.claim(2, 0, 10)
      Entry.__real_new(*args)
    end
    replacing_new(Entry, reenter) { assert_equal [1], ledger.entries.map(&:key) }
    assert_equal [1, 2], ledger.entries.map(&:key).sort
  end

  def test_every_mutating_call_refuses_a_frozen_ledger_up_front
    ledger = Ledger.new
    id = acquire(ledger, 1, 0, 10)
    ledger.freeze
    before = state(ledger)
    [
      -> { ledger.claim(2, 0, 10) }, -> { ledger.claim_with(2, 1, 0, 10) },
      -> { ledger.admit(Attempt.new(2, nil, nil), 0, 10) }, -> { ledger.complete(id, 0) },
      -> { ledger.complete_with(id, 1, 0) }, -> { ledger.abandon(id) }, -> { ledger.expire_where { true } }
    ].each { |call| assert_raises(FrozenError) { call.() } }
    assert_equal before, state(ledger)
  end
end
