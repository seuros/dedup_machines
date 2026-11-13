# dedup-machines

> **AMS Birrell**, NCC-4205, *Birrell*-class courier.

Engine-room flight manual: track repeated dispatches in a bounded delivery ledger.

Allocation-free `no_std` exact-key claim ledger. Live claims and completed keys
are retained until their original claim TTL; full tables refuse new keys instead
of evicting live records. Completion names the claim by an id that is issued
once and never again, so an expired worker cannot complete a replacement claim
for the same key.

Pending claims may expire: their workers can still be executing, so fencing or
transactional storage is required for side effects. Completion does not extend
TTL. `abandon` frees a live pending claim after a known failure. Do not abandon
an uncertain external effect and assume a retry is safe.

Completion may carry a value: `complete_with` stores it and every later claim
of that key reads it back as `Completed(&value)`. Claims may carry a
fingerprint of their payload: a live record under the same key with a different
fingerprint (or none) is refused as `Conflict`, so an id reused for another
payload is not replayed. Claims may name a partition; with a partition cap set,
a bucket at its quota is refused as `PartitionFull` while the table still has
room. `expire_where` drops records whose owner is known to be gone before their
TTL would.

```rust
use core::time::Duration;
use dedup_machines::{Claim, Completion, Dedup};
let mut ledger = Dedup::<u64, 8>::new();
let ttl = Duration::from_secs(30);
let Ok(Claim::Acquired(id)) = ledger.claim(42, Duration::ZERO, ttl)
    else { panic!("fresh key must fit"); };
assert_eq!(ledger.complete(id, Duration::ZERO), Ok(Completion::Completed));
assert_eq!(ledger.complete(id, Duration::ZERO), Ok(Completion::AlreadyCompleted));
assert_eq!(ledger.claim(42, Duration::ZERO, ttl), Ok(Claim::Completed(&())));
```

The `alloc` feature (`std` implies it) adds `HashDedup`, the same ledger over a
heap table with a runtime capacity and hashed lookup, for keys that are not
`Copy`:

```rust
# #[cfg(feature = "alloc")] {
use core::time::Duration;
use dedup_machines::{Attempt, Claim, Completion, HashDedup};
let mut ledger = HashDedup::<String, String>::with_capacity(1024).with_partition_cap(128);
let relay = |session: u64, source: &str| Attempt::new(format!("{session}/{source}")).partition(session);
let hour = Duration::from_secs(3600);
let Ok(Claim::Acquired(id)) = ledger.admit(relay(7, "tool"), Duration::ZERO, hour)
    else { panic!("fresh key must fit"); };
assert_eq!(ledger.complete_with(id, "elicit-1".into(), Duration::ZERO), Ok(Completion::Completed));
assert_eq!(
    ledger.admit(relay(7, "tool"), Duration::from_secs(1), hour),
    Ok(Claim::Completed(&"elicit-1".to_string()))
);
assert_eq!(ledger.expire_where(|key| key.starts_with("7/")), 1);
# }
```

## Contract

**Answers and refusals.** Every operation that can refuse returns
`Result<T, Refusal>`; `Refusal` is the crate's only refusal type and `T` holds
honoured outcomes only. A claim asks "may I run this key?". The ledger has
three truthful answers to that well-formed question, and all three are
honoured: `Claim::Acquired(id)` (yes), `Claim::InProgress` (no, someone holds
it) and `Claim::Completed(&value)` (no, it is done, and here is the result). A
duplicate delivery receiving `InProgress` or `Completed` is the ledger doing
its job, not turning a request away. A refusal is a question the ledger could
not answer or a request it could not carry out: a malformed call
(`TimeRegression`, `InvalidDuration`, `Overflow`), a key reused for another
payload (`Conflict`), no room (`Full`, `PartitionFull`), no id left
(`Exhausted`).

| Call | Honoured | Refusals, in the order checked |
| --- | --- | --- |
| `claim`, `claim_with` | `Acquired(ClaimId)`, `InProgress`, `Completed(&V)` | `TimeRegression`, `InvalidDuration`, `Overflow`, `Conflict`, `Exhausted`, `Full` |
| `admit` | as `claim` | as `claim`, with `PartitionFull` after `Conflict` |
| `complete` | `Completed`, `AlreadyCompleted` | `Stale` (never issued), `TimeRegression`, `Stale` (not live at `now`), `Mismatch` |
| `complete_with` | as `complete` | as `complete`, inside `Refused { refusal, value }` |
| `abandon` | `()` | `Stale`, `Concluded` |
| `expire_where` | how many it dropped (a drain: never refuses) | |

`complete_with` takes its value by move, so its error is `Refused<V>`: the one
`Refusal` plus the value handed back untouched. It is a carrier, not a second
set of reasons; `complete` has no value to return and answers a bare `Refusal`.

**A refusal changes nothing.** Not a record, not the observed clock, not an
issued id: the ledger equals a clone taken before the call. The clock is
checked without being recorded, and the id of a claim is consumed only once
its record is stored.

**Expiry is read, not swept.** A record counts at every instant before its
expiry and is read as absent at every instant from it on, by every call,
whether or not it is still held in storage. Removal is deferred to the calls
that are honoured and need it:

- `Claim::Acquired` records `now`, removes every record that expired by `now`,
  stores the new one and issues its id.
- `Claim::InProgress` and `Claim::Completed` record `now` and change nothing
  else. Recording it keeps the answer true: an earlier instant is refused
  afterwards.
- `Completion::Completed` records `now` and the value.
- `abandon` carries no clock: a claim is live for it until the *observed*
  clock reaches its expiry. It removes that one record.
- `expire_where` removes the records it was asked to drop and those that
  expired by the observed clock; it counts only the former and never shows an
  expired record to the predicate.
- No refusal removes an expired record. `Full` means every slot holds a *live*
  record at `now`.

So the answer to a call depends only on the live records and the clock, never
on which expired records happen to still be held. `Ledger::store` shows the
held ones, expired included.

**A time-based refusal is an answer about the instant supplied, not a recorded
fact.** `Stale` for an expired claim records no clock, so the same completion
at an earlier instant, one not behind the latest *honoured* call, is honoured:

```rust
use core::time::Duration;
use dedup_machines::{Claim, Completion, Dedup, Refusal};
let secs = Duration::from_secs;
let mut ledger = Dedup::<u64, 8>::new();
let Ok(Claim::Acquired(id)) = ledger.claim(42, secs(0), secs(10))
    else { panic!("fresh key must fit"); };
assert_eq!(ledger.complete(id, secs(10)), Err(Refusal::Stale));
// Nothing was recorded at 10s; 5s is not behind the observed clock (0s).
assert_eq!(ledger.complete(id, secs(5)), Ok(Completion::Completed));
```

Nothing may rest on that `Stale` being final. What ends an expired claim for
good is an honoured call that records an instant at or past its expiry: an
`Acquired` of the same key replaces the record and issues a new id (the old id
is `Stale` at every instant the ledger still accepts), and any honoured claim
or completion at such an instant makes every earlier one `TimeRegression`.
`abandon` reads the observed clock, so it too still frees a claim that was
only ever *refused* as expired.

**Repeats.** Completing a claim that is already completed, with an equal
value, while its record lives, is honoured as `Completion::AlreadyCompleted`
and changes nothing, not even the observed clock. The ledger can prove it: the
record is still there under that claim id with that value. A different value
is `Refusal::Mismatch` and the recorded value stands. Comparing needs
`V: PartialEq`, which `complete` and `complete_with` require; a value type that
cannot be compared cannot be proved equal, so it cannot be completed through
this ledger. `complete` is `complete_with(V::default())`: after a
`complete_with(id, 7)`, a bare `complete(id)` is a `Mismatch`.

A repeated `abandon` is not honoured: the slot is freed and no history is
kept, so the ledger cannot tell an abandoned claim from an expired or dropped
one. It is `Refusal::Stale`, like every claim id that names no live claim.

**Scopes.** A `ClaimId` comes from one `periscope-machines` issuer per ledger:
the ledger's scope in the top 16 bits, a sequence from 1 below, never reissued,
not when its slot is reused and not when its key is claimed again. `new()` is
scope zero; `with_scope(scope)` picks another. Two ledgers in one scope issue
the same integers, so give every ledger whose ids could meet its own scope: an
id of another scope, or one this ledger has not issued yet, is `Stale` before
any record is looked up. `ClaimId::raw` and `ClaimId::from_raw` carry an id
across a queue or a column; `ClaimId::scope` reads its scope.

**Restarts.** `Ledger::resumed(store, scope, issued, observed)` continues a
persisted ledger from `Ledger::issued` and `Ledger::observed`, so no id that
outlived the restart is issued again. It takes the scope explicitly and answers
`None` for saved state that is not that scope's: a record whose claim id
carries another scope or a sequence outside `1..=issued`, two slots carrying
the same key or the same claim id (the same record stored twice included), or
a count beyond 2^48 - 1. (The Ruby binding raises
`RangeError` where this answers `None`; it never answers `nil`.)
`Ledger::over(store)` starts
a sequence from nothing and therefore takes only an empty store. A ledger that
cannot save its count takes a scope no earlier incarnation used.

**Time.** Instants and TTLs are `core::time::Duration` on a monotonic clock the
caller owns; the ledger reads none.

## Bring Your Own Storage

`Fixed` and `Hashed` keep records in memory. `Storage` is a public trait, so
the same ledger can run over a database table or a shared-memory arena: a
backend stores the `Entry` values the ledger hands it and finds them again by
key (`find`), by claim id (`locate`) or by slot, and lists them with their
slots (`iter`). `Entry::new` and its accessors round-trip every field. Two
rules keep refusals free of side effects: an `insert` that answers `None`
stored nothing, and an `insert` right after a `retain` that dropped a record
finds room.

No two held records share a key or a claim id. `Fixed` and `Hashed` answer
`None` to an `insert` that would break that, and store nothing. A backend of
your own may not check, and any backend can be overwritten in place through
`get_mut`, so `Ledger::resumed` is the gate: it refuses a store in which some
record is not the one `find` answers for its key and `locate` answers for its
claim id.
