//! Thin binding to the exact bounded claim ledger.
mod classes;
mod convert;
mod init;

use std::{cell::RefCell, time::Duration};

use dedup_machines::{Attempt, Claim, ClaimId, Completion, Dedup, Fixed, Refusal, Storage};
use magnus::{Error, Integer, RArray, Ruby, Value, prelude::*, typed_data::Obj};
use matryona_machines::ruby::{Symbols, duration, exact_array, integer, keep, nanoseconds, symbol};

use classes::classes;
use convert::{Refusals, Stored};

const CAPACITY: usize = 64;
/// What `resumed` raises for saved state the core answers `None` to.
const UNSOUND: &str = "entries are not the saved state of this scope and count";

type Core = Dedup<u64, CAPACITY, Stored>;

#[magnus::wrap(class = "DedupMachines::Ledger", free_immediately, size)]
struct Ledger {
    core: RefCell<Core>,
}

fn refused(ruby: &Ruby, why: Refusal) -> Value {
    Refusals::symbol(ruby, why).as_value()
}

/// A `Claim` with its completed value copied out of the ledger, so the
/// answer outlives the copy it was decided on.
enum Asked {
    Acquired(ClaimId),
    InProgress,
    Completed(Stored),
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

    fn with_scope(scope: Value) -> Result<Self, Error> {
        Ok(Self::wrap(Core::with_scope(integer(scope, "scope")?)))
    }

    /// `Ledger::resumed` over the given entries. A restoring constructor has
    /// no refusal symbol to answer with: malformed input raises, and so does
    /// saved state the core refuses (it answers `None`), as `RangeError`.
    fn resumed(
        ruby: &Ruby,
        entries: Value,
        scope: Value,
        issued: Value,
        observed: Value,
    ) -> Result<Self, Error> {
        let entries = exact_array(ruby, entries, "entries must be an Array of Entry")?;
        let (scope, issued) = (integer(scope, "scope")?, integer(issued, "issued")?);
        let observed = duration(observed, "observed")?;
        let range = |message| Error::new(ruby.exception_range_error(), message);
        if issued > periscope_machines::SEQUENCE_MAX {
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
        Core::resumed(store, scope, issued, observed)
            .map(Self::wrap)
            .ok_or_else(|| range(UNSOUND))
    }

    /// The core's builder consumes the ledger; Ruby gets a capped copy.
    /// `RangeError` for a cap past `usize` (a 32-bit host): it is not
    /// rounded down to a cap the caller did not ask for.
    fn with_partition_cap(this: &Self, cap: Value) -> Result<Self, Error> {
        let cap: usize = integer(cap, "cap")?;
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
        nanoseconds(ruby, this.core.borrow().observed())
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
        key: Value,
        now: Value,
        ttl: Value,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let key = integer(key, "key")?;
        let (now, ttl) = (duration(now, "now")?, duration(ttl, "ttl")?);
        Self::ask(ruby, this, Attempt::new(key), now, ttl)
    }

    fn claim_with(
        ruby: &Ruby,
        this: Obj<Self>,
        key: Value,
        fingerprint: Value,
        now: Value,
        ttl: Value,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let (key, fingerprint) = (integer(key, "key")?, integer(fingerprint, "fingerprint")?);
        let (now, ttl) = (duration(now, "now")?, duration(ttl, "ttl")?);
        let attempt = Attempt::new(key).fingerprint(fingerprint);
        Self::ask(ruby, this, attempt, now, ttl)
    }

    fn admit(
        ruby: &Ruby,
        this: Obj<Self>,
        attempt: Value,
        now: Value,
        ttl: Value,
    ) -> Result<Value, Error> {
        this.check_frozen()?;
        let attempt = convert::attempt(ruby, attempt)?;
        let (now, ttl) = (duration(now, "now")?, duration(ttl, "ttl")?);
        Self::ask(ruby, this, attempt, now, ttl)
    }

    /// An `Acquired`, `:in_progress`, a `Completed`, or a refusal symbol.
    fn ask(
        ruby: &Ruby,
        this: Obj<Self>,
        attempt: Attempt<u64>,
        now: Duration,
        ttl: Duration,
    ) -> Result<Value, Error> {
        let classes = classes(ruby)?;
        // Decided on a copy, kept once the answer is built: after an
        // `Acquired` the claim exists, and losing its id would strand it.
        keep(
            ruby,
            this,
            &this.core,
            |core| {
                core.admit(attempt, now, ttl).map(|claim| match claim {
                    Claim::Acquired(id) => Asked::Acquired(id),
                    Claim::InProgress => Asked::InProgress,
                    Claim::Completed(value) => Asked::Completed(*value),
                })
            },
            |asked| match asked {
                Ok(Asked::Acquired(id)) => {
                    let id = convert::claim_value(ruby, classes, id)?;
                    classes.acquired.get(ruby).funcall("new", (id,))
                }
                Ok(Asked::Completed(value)) => classes.completed.get(ruby).funcall("new", (value,)),
                Ok(Asked::InProgress) => Ok(symbol(ruby, "in_progress").as_value()),
                Err(why) => Ok(refused(ruby, why)),
            },
        )
    }

    /// The claim a settling call names, once the ledger is known mutable.
    fn settling(ruby: &Ruby, this: Obj<Self>, claim: Value) -> Result<ClaimId, Error> {
        this.check_frozen()?;
        convert::claim_id(ruby, claim)
    }

    fn complete(ruby: &Ruby, this: Obj<Self>, claim: Value, now: Value) -> Result<Value, Error> {
        let claim = Self::settling(ruby, this, claim)?;
        let now = duration(now, "now")?;
        let outcome = this.core.borrow_mut().complete(claim, now);
        Ok(Self::completion(ruby, outcome))
    }

    /// The core hands the value back with the refusal; Ruby still holds its
    /// Integer, so the answer is the refusal symbol alone.
    fn complete_with(
        ruby: &Ruby,
        this: Obj<Self>,
        claim: Value,
        value: Value,
        now: Value,
    ) -> Result<Value, Error> {
        let claim = Self::settling(ruby, this, claim)?;
        let (value, now) = (integer(value, "value")?, duration(now, "now")?);
        let mut core = this.core.borrow_mut();
        let outcome = core.complete_with(claim, value, now);
        Ok(Self::completion(ruby, outcome.map_err(|back| back.refusal)))
    }

    fn completion(ruby: &Ruby, outcome: Result<Completion, Refusal>) -> Value {
        match outcome {
            Ok(Completion::Completed) => symbol(ruby, "completed").as_value(),
            Ok(Completion::AlreadyCompleted) => symbol(ruby, "already_completed").as_value(),
            Err(why) => refused(ruby, why),
        }
    }

    fn abandon(ruby: &Ruby, this: Obj<Self>, claim: Value) -> Result<Value, Error> {
        let claim = Self::settling(ruby, this, claim)?;
        Ok(match this.core.borrow_mut().abandon(claim) {
            Ok(()) => symbol(ruby, "abandoned").as_value(),
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
