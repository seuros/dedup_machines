# frozen_string_literal: true

require_relative 'dedup_machines/version'

module DedupMachines
  # Every value below validates at construction through the core, so one built
  # by hand is held to the rule a returned one is: TypeError for the wrong
  # type, RangeError for a value the core rejects. Integers are in
  # 0..2**64-1, instants Integer nanoseconds.

  # The core's `ClaimId`: the identity of one acquired claim, issued once per
  # ledger and never again. `raw` is its Integer form, the ledger's scope in
  # the top 16 bits above a sequence starting at 1, so scope zero issues 1, 2,
  # 3. `ClaimId.new(raw)` is the core's `ClaimId::from_raw`: RangeError for a
  # sequence of zero, which no ledger issues; any other Integer builds one,
  # and the ledger answers :stale to an id it never issued.
  ClaimId = Data.define(:raw) do
    def initialize(raw:)
      DedupMachines.__send__(:__claim_scope, raw)
      super
    end

    # The scope of the ledger that issued this id.
    def scope = DedupMachines.__send__(:__claim_scope, raw)
  end

  # The core's `Claim::Acquired(id)`: the key was free and the caller now owns
  # a pending claim, named by `id` (a ClaimId; TypeError otherwise). The other
  # honoured answers are the symbol :in_progress (`Claim::InProgress`) and a
  # Completed.
  Acquired = Data.define(:id) do
    def initialize(id:)
      DedupMachines.__send__(:__acquired, id)
      super
    end
  end

  # The core's `Claim::Completed(value)`: the key completed within its
  # retention interval, with this value (an Integer; 0 after a bare
  # `complete`).
  Completed = Data.define(:value) do
    def initialize(value:)
      DedupMachines.__send__(:__completed, value)
      super
    end
  end

  # The core's `Attempt`: an Integer key plus the fingerprint and the
  # partition it is claimed under, each an Integer or nil.
  Attempt = Data.define(:key, :fingerprint, :partition) do
    def initialize(key:, fingerprint:, partition:)
      DedupMachines.__send__(:__attempt, [key, fingerprint, partition])
      super
    end
  end

  # The core's `Entry`, in the order of `Entry::new`: one stored record. `claim`
  # is the ClaimId it was issued under, `expires` the first instant (Integer
  # nanoseconds) at which it no longer counts, `value` nil while the claim is
  # pending.
  Entry = Data.define(:key, :fingerprint, :partition, :claim, :expires, :value) do
    def initialize(key:, fingerprint:, partition:, claim:, expires:, value:)
      DedupMachines.__send__(:__entry, [key, fingerprint, partition, claim, expires, value])
      super
    end

    # True while the claim is uncompleted.
    def pending? = value.nil?
  end
end

# Native is required: no algorithm fallback. It builds the Data values above,
# so they must exist first.
require 'dedup_machines_native/dedup_machines_native'

module DedupMachines
  private_class_method :__claim_scope, :__acquired, :__completed, :__attempt, :__entry

  # The values are final. `initialize` is where each validates, and a subclass
  # could override it, or `allocate` build an instance that never ran it:
  # subclassing raises TypeError (as subclassing Class does) and `allocate`
  # is gone. `new`, keywords, `with` and `dup` all still run the check.
  [ClaimId, Acquired, Completed, Attempt, Entry].each do |value|
    value.singleton_class.undef_method(:allocate)
    value.define_singleton_method(:inherited) { |_| raise TypeError, "#{value} cannot be subclassed" }
  end
end
