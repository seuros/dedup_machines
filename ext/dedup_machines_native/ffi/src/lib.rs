//! Thin binding to the exact bounded claim ledger.
mod classes;
mod convert;
mod init;

use std::cell::RefCell;

use dedup_machines::{Attempt, Claim, ClaimId, Completion, Dedup, Fixed, Refusal, Storage};
use magnus::{Error, Integer, RArray, Ruby, Value, prelude::*, typed_data::Obj};

use classes::{classes, keep};
use convert::{Number, Scope, Stored, instant, refusal};

const CAPACITY: usize = 64;
/// What `resumed` raises for saved state the core answers `None` to.
const UNSOUND: &str = "entries are not the saved state of this scope and count";

type Core = Dedup<u64, CAPACITY, Stored>;

#[magnus::wrap(class = "DedupMachines::Ledger", free_immediately, size)]
struct Ledger {
    core: RefCell<Core>,
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
    /// `RangeError` for a cap past `usize` (a 32-bit host): it is not
    /// rounded down to a cap the caller did not ask for.
    fn with_partition_cap(ruby: &Ruby, this: &Self, cap: Number) -> Result<Self, Error> {
        let cap = usize::try_from(cap.0).map_err(|_| {
            Error::new(
                ruby.exception_range_error(),
                "partition cap must fit the platform's usize",
            )
        })?;
        Ok(Self::wrap(
            this.core.borrow().clone().with_partition_cap(cap),
        ))
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

    /// Every record the store holds, as frozen `Entry` values. The records
    /// are copied out first, so no borrow is held while `new` runs.
    fn entries(ruby: &Ruby, this: &Self) -> Result<RArray, Error> {
        let classes = classes(ruby)?;
        let records: Vec<convert::Entry> = {
            let core = this.core.borrow();
            core.store()
                .iter()
                .map(|(_, entry)| entry.clone())
                .collect()
        };
        let entries = ruby.ary_new_capa(records.len());
        for entry in &records {
            entries.push(convert::entry_value(ruby, classes, entry)?)?;
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
        let classes = classes(ruby)?;
        // Decided on a copy, kept once the answer is built: after an
        // `Acquired` the claim exists, and losing its id would strand it.
        keep(ruby, this.as_value(), &this.core, |core| {
            match core.admit(attempt, now.nanos(), ttl.nanos()) {
                Ok(Claim::Acquired(id)) => {
                    let id = convert::claim_value(classes, id)?;
                    classes.acquired.funcall("new", (id,))
                }
                Ok(Claim::Completed(value)) => classes.completed.funcall("new", (*value,)),
                Ok(Claim::InProgress) => Ok(ruby.to_symbol("in_progress").as_value()),
                Err(why) => Ok(refused(ruby, why)),
            }
        })
    }

    /// The claim a settling call names, once the ledger is known mutable.
    fn settling(ruby: &Ruby, this: Obj<Self>, claim: Value) -> Result<ClaimId, Error> {
        this.check_frozen()?;
        convert::claim_id(ruby, claim)
    }

    fn complete(ruby: &Ruby, this: Obj<Self>, claim: Value, now: Number) -> Result<Value, Error> {
        let claim = Self::settling(ruby, this, claim)?;
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
        let claim = Self::settling(ruby, this, claim)?;
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
        let claim = Self::settling(ruby, this, claim)?;
        Ok(match this.core.borrow_mut().abandon(claim) {
            Ok(()) => ruby.to_symbol("abandoned").as_value(),
            Err(why) => refused(ruby, why),
        })
    }

    /// The block sees each live key once, before anything is dropped and with
    /// the ledger unborrowed, so a block that raises leaves it untouched.
    /// Only a record the block was shown is dropped: if the block itself
    /// abandoned a key and claimed it again, the new claim stays.
    fn expire_where(ruby: &Ruby, this: Obj<Self>) -> Result<usize, Error> {
        this.check_frozen()?;
        let block = ruby.block_proc()?;
        let shown = Self::live(&this.core.borrow());
        let mut dead = Vec::with_capacity(shown.len());
        for (key, claim) in shown {
            let verdict: Value = block.call((key,))?;
            if verdict.to_bool() {
                dead.push((key, claim));
            }
        }
        this.check_frozen()?;
        let mut core = this.core.borrow_mut();
        // The block ran with the ledger free: keep only the verdicts on
        // records still held under the claim the block saw.
        let held = Self::live(&core);
        dead.retain(|record| held.contains(record));
        Ok(core.expire_where(|key| dead.iter().any(|(dead, _)| dead == key)))
    }

    /// Key and claim id of every record live at the observed clock.
    fn live(core: &Core) -> Vec<(u64, ClaimId)> {
        let observed = core.observed();
        core.store()
            .iter()
            .filter(|(_, entry)| entry.expires() > observed)
            .map(|(_, entry)| (*entry.key(), entry.claim()))
            .collect()
    }
}
