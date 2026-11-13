//! Ruby value conversion and refusal names.
use std::time::Duration;

use dedup_machines::{ClaimId, Refusal};
use magnus::{Error, Integer, RClass, Ruby, TryConvert, Value, prelude::*};

/// Values the ledger stores on completion: Integers, like its keys.
pub type Stored = u64;
pub type Entry = dedup_machines::Entry<u64, Stored>;

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

pub fn data(ruby: &Ruby, name: &str) -> Result<RClass, Error> {
    ruby.define_module("DedupMachines")?.const_get(name)
}

/// `value` as an instance of the gem's `name` Data class.
fn instance(ruby: &Ruby, value: Value, name: &str) -> Result<Value, Error> {
    if value.is_kind_of(data(ruby, name)?) {
        return Ok(value);
    }
    Err(Error::new(
        ruby.exception_type_error(),
        format!("expected a DedupMachines::{name}"),
    ))
}

fn optional(value: Value) -> Result<Option<u64>, Error> {
    if value.is_nil() {
        return Ok(None);
    }
    Number::try_convert(value).map(|number| Some(number.0))
}

/// The core id a Ruby `ClaimId` names.
pub fn claim_id(ruby: &Ruby, value: Value) -> Result<ClaimId, Error> {
    let raw: Number = instance(ruby, value, "ClaimId")?.funcall("raw", ())?;
    Ok(ClaimId::from_raw(raw.0))
}

/// A Ruby `ClaimId` for a core id.
pub fn claim_value(ruby: &Ruby, id: ClaimId) -> Result<Value, Error> {
    data(ruby, "ClaimId")?.funcall("new", (id.raw(),))
}

/// The key, fingerprint and partition of a Ruby `Attempt`.
pub fn attempt(ruby: &Ruby, value: Value) -> Result<dedup_machines::Attempt<u64>, Error> {
    let value = instance(ruby, value, "Attempt")?;
    let key: Number = value.funcall("key", ())?;
    let mut attempt = dedup_machines::Attempt::new(key.0);
    if let Some(fingerprint) = optional(value.funcall("fingerprint", ())?)? {
        attempt = attempt.fingerprint(fingerprint);
    }
    if let Some(partition) = optional(value.funcall("partition", ())?)? {
        attempt = attempt.partition(partition);
    }
    Ok(attempt)
}

/// The core record a Ruby `Entry` describes.
pub fn entry(ruby: &Ruby, value: Value) -> Result<Entry, Error> {
    let value = instance(ruby, value, "Entry")?;
    let key: Number = value.funcall("key", ())?;
    Ok(Entry::new(
        key.0,
        optional(value.funcall("fingerprint", ())?)?,
        optional(value.funcall("partition", ())?)?,
        claim_id(ruby, value.funcall("claim", ())?)?,
        expiry(ruby, value.funcall("expires", ())?)?,
        optional(value.funcall("value", ())?)?,
    ))
}

/// A Ruby `Entry` for a core record.
pub fn entry_value(ruby: &Ruby, entry: &Entry) -> Result<Value, Error> {
    let fields = (
        *entry.key(),
        entry.fingerprint(),
        entry.partition(),
        claim_value(ruby, entry.claim())?,
        instant(ruby, entry.expires()),
        entry.value().copied(),
    );
    data(ruby, "Entry")?.funcall("new", fields)
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
