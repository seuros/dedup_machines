//! Ruby value conversion and refusal names.
use dedup_machines::{ClaimId, Refusal};
use magnus::{Error, Ruby, Value, prelude::*};
use matryona_machines::ruby::{
    exact_array_n, exact_fields, instant, integer, nanoseconds, optional,
};

use crate::classes::{Classes, classes};

/// Values the ledger stores on completion: Integers, like its keys.
pub type Stored = u64;
pub type Entry = dedup_machines::Entry<u64, Stored>;

/// The core id an Integer names: `ClaimId::from_raw`, `RangeError` for a
/// sequence of zero, which no ledger issues.
pub fn raw_claim_id(ruby: &Ruby, raw: Value) -> Result<ClaimId, Error> {
    ClaimId::from_raw(integer(raw, "raw")?).ok_or_else(|| {
        Error::new(
            ruby.exception_range_error(),
            "a claim id's sequence (the low 48 bits) must not be zero",
        )
    })
}

/// The core id a Ruby `ClaimId` names. It must be exactly a `ClaimId`
/// (`TypeError` otherwise, a subclass included); its `raw` is read natively
/// and checked again (`RangeError`), so a forged one never reaches the core.
pub fn claim_id(ruby: &Ruby, value: Value) -> Result<ClaimId, Error> {
    let class = classes(ruby)?.claim_id.get(ruby);
    let [raw] = exact_fields::<1>(ruby, value, class, "expected a DedupMachines::ClaimId")?;
    raw_claim_id(ruby, raw)
}

/// What `ClaimId.new` stores, and `ClaimId#scope`: the scope of a checked id.
pub fn claim_scope(ruby: &Ruby, raw: Value) -> Result<u16, Error> {
    raw_claim_id(ruby, raw).map(ClaimId::scope)
}

/// What `Acquired.new` checks: `id` is a `ClaimId`.
pub fn check_acquired(ruby: &Ruby, id: Value) -> Result<(), Error> {
    claim_id(ruby, id).map(drop)
}

/// What `Completed.new` checks: `value` is an Integer in `0..2**64-1`.
pub fn check_completed(value: Value) -> Result<(), Error> {
    integer::<u64>(value, "value").map(drop)
}

/// What `Attempt.new` checks: an Integer key, each of fingerprint and
/// partition an Integer or nil, all in `0..2**64-1`.
pub fn check_attempt(ruby: &Ruby, fields: Value) -> Result<(), Error> {
    let [key, fingerprint, partition] = row(ruby, fields)?;
    integer::<u64>(key, "key")?;
    optional::<u64>(fingerprint, "fingerprint")?;
    optional::<u64>(partition, "partition").map(drop)
}

/// `Entry`'s six fields, in `Entry::new` order.
type Fields = [Value; 6];

/// What `Entry.new` checks: the fields `Entry::new` takes, in its order.
pub fn check_entry(ruby: &Ruby, fields: Value) -> Result<(), Error> {
    entry_from(ruby, row(ruby, fields)?).map(drop)
}

/// The `N` fields a value's `new` hands its hook, exactly an `Array` of `N`.
fn row<const N: usize>(ruby: &Ruby, fields: Value) -> Result<[Value; N], Error> {
    exact_array_n::<N>(ruby, fields, format!("fields must be an Array of {N}"))
}

/// A Ruby `ClaimId` for a core id.
pub fn claim_value(ruby: &Ruby, classes: &Classes, id: ClaimId) -> Result<Value, Error> {
    classes.claim_id.get(ruby).funcall("new", (id.raw(),))
}

/// The key, fingerprint and partition of a Ruby `Attempt`: exactly an
/// `Attempt`, its fields read natively and checked again.
pub fn attempt(ruby: &Ruby, value: Value) -> Result<dedup_machines::Attempt<u64>, Error> {
    let class = classes(ruby)?.attempt.get(ruby);
    let [key, fingerprint, partition] =
        exact_fields::<3>(ruby, value, class, "expected a DedupMachines::Attempt")?;
    let mut attempt = dedup_machines::Attempt::new(integer(key, "key")?);
    if let Some(fingerprint) = optional(fingerprint, "fingerprint")? {
        attempt = attempt.fingerprint(fingerprint);
    }
    if let Some(partition) = optional(partition, "partition")? {
        attempt = attempt.partition(partition);
    }
    Ok(attempt)
}

/// The core record a Ruby `Entry` describes: exactly an `Entry`, its six
/// fields read natively and checked again.
pub fn entry(ruby: &Ruby, value: Value) -> Result<Entry, Error> {
    let class = classes(ruby)?.entry.get(ruby);
    let fields = exact_fields::<6>(ruby, value, class, "expected a DedupMachines::Entry")?;
    entry_from(ruby, fields)
}

/// The core record built from `Entry`'s six fields, in `Entry::new` order.
fn entry_from(ruby: &Ruby, fields: Fields) -> Result<Entry, Error> {
    let [key, fingerprint, partition, claim, expires, value] = fields;
    Ok(Entry::new(
        integer(key, "key")?,
        optional(fingerprint, "fingerprint")?,
        optional(partition, "partition")?,
        claim_id(ruby, claim)?,
        instant(expires, "expires")?,
        optional(value, "value")?,
    ))
}

/// A Ruby `Entry` for a core record.
pub fn entry_value(ruby: &Ruby, classes: &Classes, entry: &Entry) -> Result<Value, Error> {
    let fields = (
        *entry.key(),
        entry.fingerprint(),
        entry.partition(),
        claim_value(ruby, classes, entry.claim())?,
        nanoseconds(ruby, entry.expires()),
        entry.value().copied(),
    );
    classes.entry.get(ruby).funcall("new", fields)
}

matryona_machines::symbols! {
    /// The core's refusals as Symbols: the snake case of each variant.
    pub Refusals for Refusal as "a refusal" {
        TimeRegression => "time_regression",
        InvalidDuration => "invalid_duration",
        Overflow => "overflow",
        Exhausted => "exhausted",
        Conflict => "conflict",
        Full => "full",
        PartitionFull => "partition_full",
        Stale => "stale",
        Concluded => "concluded",
        Mismatch => "mismatch",
    }
}
