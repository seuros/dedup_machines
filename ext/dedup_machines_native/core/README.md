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

This initial in-memory ledger has no persistence, cached response payloads,
cross-process arbitration, or wire checkpoint format. Handles are scoped to one
ledger lifetime; replacing the ledger resets its namespace.

```rust
use core::time::Duration;
use dedup_machines::{Claim, Dedup};
let mut ledger = Dedup::<u64, 8>::new();
let Claim::Acquired(id) = ledger.claim(42, Duration::ZERO, Duration::from_secs(30))
    else { panic!("fresh key must fit"); };
assert!(ledger.complete(id, Duration::ZERO));
assert_eq!(ledger.claim(42, Duration::ZERO, Duration::from_secs(30)), Claim::Completed);
```
