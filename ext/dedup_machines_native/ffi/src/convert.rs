//! Ruby value conversion and refusal names.
use std::time::Duration;

use dedup_machines::{ClaimId, Refusal};
use magnus::{Error, Integer, RArray, Ruby, TryConvert, Value, prelude::*};

use crate::classes::{Classes, classes, exact};

/// Values the ledger stores on completion: Integers, like its keys.
pub type Stored = u64;
pub type Entry = dedup_machines::Entry<u64, Stored>;

pub fn wrong_type(ruby: &Ruby, message: &'static str) -> Error {
    Error::new(ruby.exception_type_error(), message)
}

#[derive(Clone, Copy)]
pub struct Number(pub u64);

impl TryConvert for Number {
    fn try_convert(value: Value) -> Result<Self, Error> {
        if Integer::from_value(value).is_none() {
            return Err(Error::new(
                Ruby::get_with(value).exception_type_error(),
                "expected an Integer (unsigned 64-bit)",
            ));
        }
        u64::try_convert(value).map(Self)
    }
}

impl Number {
    /// Instants and spans cross the boundary as Integer nanoseconds.
    pub const fn nanos(self) -> Duration {
        Duration::from_nanos(self.0)
    }
}

/// A scope: an Integer in `0..=65535`.
#[derive(Clone, Copy)]
pub struct Scope(pub u16);

impl TryConvert for Scope {
    fn try_convert(value: Value) -> Result<Self, Error> {
        let ruby = Ruby::get_with(value);
        if Integer::from_value(value).is_none() {
            return Err(Error::new(
                ruby.exception_type_error(),
                "scope must be an Integer",
            ));
        }
        u16::try_convert(value)
            .map(Self)
            .map_err(|_| Error::new(ruby.exception_range_error(), "scope must be in 0..=65535"))
    }
}

/// An instant as Integer nanoseconds. Exact: `now + ttl` may pass `u64`.
pub fn instant(ruby: &Ruby, at: Duration) -> Integer {
    ruby.integer_from_u128(at.as_nanos())
}

/// An expiry read back from an `Entry`: Integer nanoseconds, possibly past
/// `u64`, within what a `Duration` holds.
fn expiry(ruby: &Ruby, value: Value) -> Result<Duration, Error> {
    const PER_SECOND: u128 = 1_000_000_000;
    if Integer::from_value(value).is_none() {
        return Err(Error::new(
            ruby.exception_type_error(),
            "expires must be an Integer",
        ));
    }
    let range = || Error::new(ruby.exception_range_error(), "expires is out of range");
    let nanos = u128::try_convert(value).map_err(|_| range())?;
    let seconds = u64::try_from(nanos / PER_SECOND).map_err(|_| range())?;
    let rest = u32::try_from(nanos % PER_SECOND).map_err(|_| range())?;
    Ok(Duration::new(seconds, rest))
}

fn optional(value: Value) -> Result<Option<u64>, Error> {
    if value.is_nil() {
        return Ok(None);
    }
    Number::try_convert(value).map(|number| Some(number.0))
}

/// The core id an Integer names: `ClaimId::from_raw`, `RangeError` for a
/// sequence of zero, which no ledger issues.
pub fn raw_claim_id(ruby: &Ruby, raw: Value) -> Result<ClaimId, Error> {
    let raw = Number::try_convert(raw)?;
    ClaimId::from_raw(raw.0).ok_or_else(|| {
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
    let message = "expected a DedupMachines::ClaimId";
    let fields = exact(ruby, value, classes(ruby)?.claim_id, message)?;
    raw_claim_id(ruby, fields.aref(0)?)
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
    Number::try_convert(value).map(drop)
}

/// What `Attempt.new` checks: an Integer key, each of fingerprint and
/// partition an Integer or nil, all in `0..2**64-1`.
pub fn check_attempt(fields: (Value, Value, Value)) -> Result<(), Error> {
    let (key, fingerprint, partition) = fields;
    Number::try_convert(key)?;
    optional(fingerprint)?;
    optional(partition).map(drop)
}

/// `Entry`'s six fields, in `Entry::new` order.
type Fields = (Value, Value, Value, Value, Value, Value);

/// What `Entry.new` checks: the fields `Entry::new` takes, in its order.
pub fn check_entry(ruby: &Ruby, fields: RArray) -> Result<(), Error> {
    entry_from(ruby, TryConvert::try_convert(fields.as_value())?).map(drop)
}

/// A Ruby `ClaimId` for a core id.
pub fn claim_value(classes: Classes, id: ClaimId) -> Result<Value, Error> {
    classes.claim_id.funcall("new", (id.raw(),))
}

/// The key, fingerprint and partition of a Ruby `Attempt`: exactly an
/// `Attempt`, its fields read natively and checked again.
pub fn attempt(ruby: &Ruby, value: Value) -> Result<dedup_machines::Attempt<u64>, Error> {
    let message = "expected a DedupMachines::Attempt";
    let fields = exact(ruby, value, classes(ruby)?.attempt, message)?;
    let key: Number = fields.aref(0)?;
    let mut attempt = dedup_machines::Attempt::new(key.0);
    if let Some(fingerprint) = optional(fields.aref(1)?)? {
        attempt = attempt.fingerprint(fingerprint);
    }
    if let Some(partition) = optional(fields.aref(2)?)? {
        attempt = attempt.partition(partition);
    }
    Ok(attempt)
}

/// The core record a Ruby `Entry` describes: exactly an `Entry`, its six
/// fields read natively and checked again.
pub fn entry(ruby: &Ruby, value: Value) -> Result<Entry, Error> {
    let fields = exact(
        ruby,
        value,
        classes(ruby)?.entry,
        "expected a DedupMachines::Entry",
    )?;
    let read = |index: usize| fields.aref::<_, Value>(index);
    entry_from(
        ruby,
        (read(0)?, read(1)?, read(2)?, read(3)?, read(4)?, read(5)?),
    )
}

/// The core record built from `Entry`'s six fields, in `Entry::new` order.
fn entry_from(ruby: &Ruby, fields: Fields) -> Result<Entry, Error> {
    let (key, fingerprint, partition, claim, expires, value) = fields;
    Ok(Entry::new(
        Number::try_convert(key)?.0,
        optional(fingerprint)?,
        optional(partition)?,
        claim_id(ruby, claim)?,
        expiry(ruby, expires)?,
        optional(value)?,
    ))
}

/// A Ruby `Entry` for a core record.
pub fn entry_value(ruby: &Ruby, classes: Classes, entry: &Entry) -> Result<Value, Error> {
    let fields = (
        *entry.key(),
        entry.fingerprint(),
        entry.partition(),
        claim_value(classes, entry.claim())?,
        instant(ruby, entry.expires()),
        entry.value().copied(),
    );
    classes.entry.funcall("new", fields)
}

/// Ruby symbol name of a refusal: the snake case of the core variant.
pub const fn refusal(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::TimeRegression => "time_regression",
        Refusal::InvalidDuration => "invalid_duration",
        Refusal::Overflow => "overflow",
        Refusal::Exhausted => "exhausted",
        Refusal::Conflict => "conflict",
        Refusal::Full => "full",
        Refusal::PartitionFull => "partition_full",
        Refusal::Stale => "stale",
        Refusal::Concluded => "concluded",
        Refusal::Mismatch => "mismatch",
    }
}
