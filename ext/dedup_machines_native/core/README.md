# dedup-machines

> **AMS Birrell**, NCC-4205, *Birrell*-class courier.

Engine-room flight manual: track repeated dispatches in a bounded delivery ledger.

Allocation-free `no_std` exact-key claim ledger. Live claims and completed keys
are retained until their original claim TTL; full tables reject new keys instead
of evicting live records. Completion is generation-checked, so an expired worker
cannot complete a replacement claim for the same slot/key.

Time is explicit and monotonic. Zero/overflowing TTLs and backwards time are
rejected. Pending claims may expire: their workers can still be executing, so
fencing or transactional storage is required for side effects. Completion does
not extend TTL. `abandon` frees a matching pending claim after a known failure.
Do not abandon an uncertain external effect and assume a retry is safe.

Completion may carry a value: `complete_with` stores it and every later claim
of that key reads it back as `Completed(&value)`. Claims may carry a
fingerprint of their payload: a record under the same key with a different
fingerprint (or none) answers `Conflict`, so an id reused for another payload is
refused instead of replayed. Claims may name a partition; with a partition cap
set, a bucket at its quota answers `PartitionFull` while the table still has
room. `expire_where` drops records whose owner is known to be gone before their
TTL would.

This in-memory ledger has no persistence, cross-process arbitration, or wire
checkpoint format. Handles are scoped to one ledger lifetime; replacing the
ledger resets its namespace.

```rust
use core::time::Duration;
use dedup_machines::{Claim, Dedup};
let mut ledger = Dedup::<u64, 8>::new();
let Claim::Acquired(id) = ledger.claim(42, Duration::ZERO, Duration::from_secs(30))
    else { panic!("fresh key must fit"); };
assert!(ledger.complete(id, Duration::ZERO));
assert_eq!(ledger.claim(42, Duration::ZERO, Duration::from_secs(30)), Claim::Completed(&()));
```

The `alloc` feature (`std` implies it) adds `HashDedup`, the same ledger over a
heap table with a runtime capacity and hashed lookup, for keys that are not
`Copy`:

```rust
# #[cfg(feature = "alloc")] {
use core::time::Duration;
use dedup_machines::{Attempt, Claim, HashDedup};
let mut ledger = HashDedup::<String, String>::with_capacity(1024).with_partition_cap(128);
let relay = |session: u64, source: &str| Attempt::new(format!("{session}/{source}")).partition(session);
let Claim::Acquired(id) = ledger.admit(relay(7, "tool"), Duration::ZERO, Duration::from_secs(3600))
    else { panic!("fresh key must fit"); };
assert_eq!(ledger.complete_with(id, "elicit-1".into(), Duration::ZERO), Ok(()));
assert_eq!(
    ledger.admit(relay(7, "tool"), Duration::from_secs(1), Duration::from_secs(3600)),
    Claim::Completed(&"elicit-1".to_string())
);
assert_eq!(ledger.expire_where(|key| key.starts_with("7/")), 1);
# }
```

## Bring Your Own Storage

`Fixed` and `Hashed` keep records in memory. `Storage` is a public trait, so
the same ledger can run over a database table or a shared-memory arena: a
backend stores the `Entry` values the ledger hands it and finds them again by
key or slot. `Entry::new` and its accessors round-trip every field, and
`Ledger::restore` continues from a persisted generation and clock watermark so
no stored claim handle can be reissued.

