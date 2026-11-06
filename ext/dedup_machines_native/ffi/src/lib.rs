//! Thin binding to the exact bounded claim ledger.
mod init;

use std::{cell::RefCell, sync::Arc, time::Duration};

use dedup_machines::{Claim, ClaimId, Dedup};
use magnus::{Error, Integer, Ruby, TryConvert, Value, prelude::*, typed_data::Obj};

const CAPACITY: usize = 64;

#[derive(Clone, Copy)]
struct Number(u64);

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

#[magnus::wrap(class = "DedupMachines::Claim", free_immediately, size)]
struct Handle {
    owner: Arc<()>,
    id: ClaimId,
}

#[magnus::wrap(class = "DedupMachines::Ledger", free_immediately, size)]
struct Ledger {
    owner: Arc<()>,
    core: RefCell<Dedup<u64, CAPACITY>>,
}

impl Ledger {
    fn new() -> Self {
        Self {
            owner: Arc::new(()),
            core: RefCell::new(Dedup::new()),
        }
    }

    fn claim(
        ruby: &Ruby,
        this: Obj<Self>,
        key: Number,
        now_ns: Number,
        ttl_ns: Number,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let outcome = this.core.borrow_mut().claim(
            key.0,
            Duration::from_nanos(now_ns.0),
            Duration::from_nanos(ttl_ns.0),
        );
        Ok(match outcome {
            Claim::Acquired(id) => ruby
                .obj_wrap(Handle {
                    owner: Arc::clone(&this.owner),
                    id,
                })
                .as_value(),
            Claim::InProgress => ruby.to_symbol("in_progress").as_value(),
            Claim::Completed => ruby.to_symbol("completed").as_value(),
            Claim::Full => ruby.to_symbol("full").as_value(),
            _ => ruby.to_symbol("rejected").as_value(),
        })
    }

    fn complete(this: Obj<Self>, handle: Obj<Handle>, now_ns: Number) -> Result<bool, Error> {
        this.check_frozen()?;
        if !Arc::ptr_eq(&this.owner, &handle.owner) {
            return Ok(false);
        }
        Ok(this
            .core
            .borrow_mut()
            .complete(handle.id, Duration::from_nanos(now_ns.0)))
    }

    fn abandon(this: Obj<Self>, handle: Obj<Handle>) -> Result<bool, Error> {
        this.check_frozen()?;
        if !Arc::ptr_eq(&this.owner, &handle.owner) {
            return Ok(false);
        }
        Ok(this.core.borrow_mut().abandon(handle.id))
    }

    const fn capacity() -> usize {
        CAPACITY
    }
}
