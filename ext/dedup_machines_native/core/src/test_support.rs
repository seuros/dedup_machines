//! Helpers shared by the crate's test modules.

use core::fmt::Debug;
use core::time::Duration;

use std::panic::{AssertUnwindSafe, catch_unwind};

use matryona_machines::testing::SplitMix64;

use crate::{Claim, ClaimId, Entry, Ledger, Refusal, Storage};

pub fn at(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

pub fn acquired<V: Debug>(claim: Result<Claim<'_, V>, Refusal>) -> ClaimId {
    match claim {
        Ok(Claim::Acquired(id)) => id,
        other => panic!("expected an acquired claim, got {other:?}"),
    }
}

/// A draw in `0..bound`, as an index.
pub fn pick(rng: &mut SplitMix64, bound: usize) -> usize {
    usize::try_from(rng.below(bound as u64)).unwrap_or(0)
}

/// A claim id from its integer form; panics on a sequence of zero.
pub fn id(raw: u64) -> ClaimId {
    ClaimId::from_raw(raw).unwrap_or_else(|| panic!("{raw} has a sequence of zero"))
}

/// A pending record of `key` claimed under `raw`, live until 9.
pub fn record<V>(key: u64, raw: u64) -> Entry<u64, V> {
    Entry::new(key, None, None, id(raw), at(9), None)
}

/// The `Storage` contract of an in-crate backend, from an empty `store`: one
/// record per key and per claim id, completing the one change a record sees.
pub fn holds_the_storage_contract<S>(mut store: S)
where
    S: Storage<Key = u64, Value = u8> + Clone + PartialEq + Debug,
{
    assert_eq!(store.insert(record(1, 1)), Some(0));
    let before = store.clone();
    // The same record, the same key, the same claim id: nothing is stored.
    for second in [record(1, 1), record(1, 2), record(2, 1)] {
        assert_eq!(store.insert(second), None);
        assert_eq!(store, before);
    }
    // No record at the slot, or no such slot: the value comes back.
    assert_eq!(store.complete(1, 7), Err(7));
    assert_eq!(store.complete(9, 7), Err(7));
    assert_eq!(store, before);
    assert_eq!(store.complete(0, 7), Ok(()));
    // A completed record keeps its value, and both lookups still lead to it.
    let before = store.clone();
    assert_eq!(store.complete(0, 8), Err(8));
    assert_eq!(store, before);
    assert_eq!((store.find(&1), store.locate(id(1))), (Some(0), Some(0)));
    assert_eq!(store.get(0).and_then(Entry::value), Some(&7));
    assert_eq!(store.insert(record(2, 2)), Some(1));
    assert!(Ledger::resumed(store, 0, 2, at(0)).is_some());
}

/// The defect: `expire_where` dropped records as it asked about them, so a
/// predicate that unwound part-way left the earlier verdicts applied. Over
/// `ledger`, empty and holding at least two records, nothing is dropped.
pub fn an_unwinding_predicate_drops_nothing<S>(mut ledger: Ledger<S>)
where
    S: Storage<Key = u64> + Clone + PartialEq + Debug,
    S::Value: Debug,
{
    for key in [1, 2] {
        acquired(ledger.claim(key, at(0), at(10)));
    }
    let before = ledger.clone();
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        ledger.expire_where(|&key| key != 2 || panic!("the predicate unwinds on key 2"))
    }));
    assert!(unwound.is_err());
    assert_eq!(ledger, before);
    assert_eq!(ledger.expire_where(|&key| key != 2), 1);
}
