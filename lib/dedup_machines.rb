# frozen_string_literal: true

require_relative 'dedup_machines/version'

module DedupMachines
  # The core's `ClaimId`: the identity of one acquired claim, issued once per
  # ledger and never again. `raw` is its Integer form, the ledger's scope in
  # the top 16 bits above a sequence starting at 1, so scope zero issues 1, 2,
  # 3. `ClaimId.new(raw)` is the core's `ClaimId::from_raw`: any Integer builds
  # one, and the ledger answers :stale to an id it never issued.
  ClaimId = Data.define(:raw) do
    # The scope of the ledger that issued this id.
    def scope = Ledger.__send__(:__scope_of, raw)
  end

  # The core's `Claim::Acquired(id)`: the key was free and the caller now owns
  # a pending claim, named by `id` (a ClaimId). The other honoured answers are
  # the symbol :in_progress (`Claim::InProgress`) and a Completed.
  Acquired = Data.define(:id)

  # The core's `Claim::Completed(value)`: the key completed within its
  # retention interval, with this value (an Integer; 0 after a bare
  # `complete`).
  Completed = Data.define(:value)

  # The core's `Attempt`: a key plus the fingerprint and the partition it is
  # claimed under, each an Integer or nil.
  Attempt = Data.define(:key, :fingerprint, :partition)

  # The core's `Entry`, in the order of `Entry::new`: one stored record. `claim`
  # is the ClaimId it was issued under, `expires` the first instant (Integer
  # nanoseconds) at which it no longer counts, `value` nil while the claim is
  # pending.
  Entry = Data.define(:key, :fingerprint, :partition, :claim, :expires, :value) do
    # True while the claim is uncompleted.
    def pending? = value.nil?
  end
end

# Native is required: no algorithm fallback. It builds the Data values above,
# so they must exist first.
require 'dedup_machines_native/dedup_machines_native'

module DedupMachines
  class Ledger
    private_class_method :__scope_of
  end
end
