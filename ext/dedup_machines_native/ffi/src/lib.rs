//! Thin binding to the exact bounded claim ledger.
mod convert;
mod init;

use std::cell::RefCell;

use dedup_machines::{Attempt, Claim, ClaimId, Completion, Dedup, Fixed, Refusal, Storage};
use magnus::{Error, Integer, RArray, Ruby, Value, prelude::*, typed_data::Obj};

use convert::{Number, Scope, Stored, data, instant, refusal};

const CAPACITY: usize = 64;
/// What `resumed` raises for saved state the core answers `None` to.
const UNSOUND: &str = "entries are not the saved state of this scope and count";

type Core = Dedup<u64, CAPACITY, Stored>;

#[magnus::wrap(class = "DedupMachines::Ledger", free_immediately, size)]
struct Ledger {
    core: RefCell<Core>,
}

/// The scope a claim id carries: `ClaimId#scope`.
const fn scope_of(raw: Number) -> u16 {
    ClaimId::from_raw(raw.0).scope()
}

fn refused(ruby: &Ruby, why: Refusal) -> Value {
    ruby.to_symbol(refusal(why)).as_value()
}

// Refusals are answers (symbols), not exceptions. Only malformed input raises.
impl Ledger {
    const fn wrap(core: Core) -> Self {
        Self {
            core: RefCell::new(core),
        }
    }

    const fn new() -> Self {
        Self::wrap(Core::new())
    }

    const fn with_scope(scope: Scope) -> Self {
        Self::wrap(Core::with_scope(scope.0))
    }

    /// `Ledger::resumed` over the given entries. A restoring constructor has
    /// no refusal symbol to answer with: malformed input raises, and so does
    /// saved state the core refuses (it answers `None`), as `RangeError`.
    fn resumed(
        ruby: &Ruby,
        entries: RArray,
        scope: Scope,
        issued: Number,
        observed: Number,
    ) -> Result<Self, Error> {
        let range = |message| Error::new(ruby.exception_range_error(), message);
        if issued.0 > periscope_machines::SEQUENCE_MAX {
            return Err(range("issued must be in 0..2**48-1"));
        }
        let mut store = Fixed::new();
        for value in entries {
            let entry = convert::entry(ruby, value)?;
            // `None` is a full store, or a second record under one key or
            // claim id: saved state the core's `resumed` refuses as well.
            if store.insert(entry).is_none() {
                return Err(range(if store.iter().count() < CAPACITY {
                    UNSOUND
                } else {
                    "entries must hold at most 64 records"
                }));
            }
        }
        Core::resumed(store, scope.0, issued.0, observed.nanos())
            .map(Self::wrap)
            .ok_or_else(|| range(UNSOUND))
    }

    /// The core's builder consumes the ledger; Ruby gets a capped copy.
    fn with_partition_cap(&self, cap: Number) -> Self {
        let cap = usize::try_from(cap.0).unwrap_or(usize::MAX);
        Self::wrap(self.core.borrow().clone().with_partition_cap(cap))
    }

    fn scope(&self) -> u16 {
        self.core.borrow().scope()
    }

    fn issued(&self) -> u64 {
        self.core.borrow().issued()
    }

    fn observed(ruby: &Ruby, this: &Self) -> Integer {
        instant(ruby, this.core.borrow().observed())
    }

    fn partition_cap(&self) -> Option<usize> {
        self.core.borrow().partition_cap()
    }

    const fn capacity() -> usize {
        CAPACITY
    }

    /// Every record the store holds, as frozen `Entry` values.
    fn entries(ruby: &Ruby, this: &Self) -> Result<RArray, Error> {
        let core = this.core.borrow();
        let entries = ruby.ary_new_capa(CAPACITY);
        for (_, entry) in core.store().iter() {
            entries.push(convert::entry_value(ruby, entry)?)?;
        }
        entries.freeze();
        Ok(entries)
    }

    fn claim(
        ruby: &Ruby,
        this: Obj<Self>,
        key: Number,
        now: Number,
        ttl: Number,
    ) -> Result<Value, Error> {
        Self::ask(ruby, this, Attempt::new(key.0), now, ttl)
    }

    fn claim_with(
        ruby: &Ruby,
        this: Obj<Self>,
        key: Number,
        fingerprint: Number,
        now: Number,
        ttl: Number,
    ) -> Result<Value, Error> {
        let attempt = Attempt::new(key.0).fingerprint(fingerprint.0);
        Self::ask(ruby, this, attempt, now, ttl)
    }

    fn admit(
        ruby: &Ruby,
        this: Obj<Self>,
        attempt: Value,
        now: Number,
        ttl: Number,
    ) -> Result<Value, Error> {
        let attempt = convert::attempt(ruby, attempt)?;
        Self::ask(ruby, this, attempt, now, ttl)
    }

    /// An `Acquired`, `:in_progress`, a `Completed`, or a refusal symbol.
    fn ask(
        ruby: &Ruby,
        this: Obj<Self>,
        attempt: Attempt<u64>,
        now: Number,
        ttl: Number,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let mut core = this.core.borrow_mut();
        // Copy the answer out so the borrow ends before Ruby is called.
        let answer = match core.admit(attempt, now.nanos(), ttl.nanos()) {
            Ok(Claim::Acquired(id)) => Ok(Ok(id)),
            Ok(Claim::Completed(value)) => Ok(Err(Some(*value))),
            Ok(Claim::InProgress) => Ok(Err(None)),
            Err(why) => Err(why),
        };
        drop(core);
        match answer {
            Ok(Ok(id)) => {
                let id = convert::claim_value(ruby, id)?;
                data(ruby, "Acquired")?.funcall("new", (id,))
            }
            Ok(Err(Some(value))) => data(ruby, "Completed")?.funcall("new", (value,)),
            Ok(Err(None)) => Ok(ruby.to_symbol("in_progress").as_value()),
            Err(why) => Ok(refused(ruby, why)),
        }
    }

    fn complete(ruby: &Ruby, this: Obj<Self>, claim: Value, now: Number) -> Result<Value, Error> {
        this.check_frozen()?;
        let claim = convert::claim_id(ruby, claim)?;
        let outcome = this.core.borrow_mut().complete(claim, now.nanos());
        Ok(Self::completion(ruby, outcome))
    }

    /// The core hands the value back with the refusal; Ruby still holds its
    /// Integer, so the answer is the refusal symbol alone.
    fn complete_with(
        ruby: &Ruby,
        this: Obj<Self>,
        claim: Value,
        value: Number,
        now: Number,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let claim = convert::claim_id(ruby, claim)?;
        let mut core = this.core.borrow_mut();
        let outcome = core.complete_with(claim, value.0, now.nanos());
        Ok(Self::completion(ruby, outcome.map_err(|back| back.refusal)))
    }

    fn completion(ruby: &Ruby, outcome: Result<Completion, Refusal>) -> Value {
        match outcome {
            Ok(Completion::Completed) => ruby.to_symbol("completed").as_value(),
            Ok(Completion::AlreadyCompleted) => ruby.to_symbol("already_completed").as_value(),
            Err(why) => refused(ruby, why),
        }
    }

    fn abandon(ruby: &Ruby, this: Obj<Self>, claim: Value) -> Result<Value, Error> {
        this.check_frozen()?;
        let claim = convert::claim_id(ruby, claim)?;
        Ok(match this.core.borrow_mut().abandon(claim) {
            Ok(()) => ruby.to_symbol("abandoned").as_value(),
            Err(why) => refused(ruby, why),
        })
    }

    /// The block sees each live key once, before anything is dropped and with
    /// the ledger unborrowed, so a block that raises leaves it untouched.
    fn expire_where(ruby: &Ruby, this: Obj<Self>) -> Result<usize, Error> {
        this.check_frozen()?;
        let block = ruby.block_proc()?;
        let core = this.core.borrow();
        let observed = core.observed();
        let live = core
            .store()
            .iter()
            .filter(|(_, entry)| entry.expires() > observed);
        let keys: Vec<u64> = live.map(|(_, entry)| *entry.key()).collect();
        drop(core);
        let mut dead = Vec::with_capacity(keys.len());
        for key in keys {
            let verdict: Value = block.call((key,))?;
            if verdict.to_bool() {
                dead.push(key);
            }
        }
        let mut core = this.core.borrow_mut();
        Ok(core.expire_where(|key| dead.contains(key)))
    }
}
