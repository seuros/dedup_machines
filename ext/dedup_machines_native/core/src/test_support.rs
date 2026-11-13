//! Helpers shared by the crate's test modules.

use core::fmt::Debug;
use core::time::Duration;

use crate::{Claim, ClaimId, Entry, Refusal, Storage};

pub fn at(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

pub fn acquired<V: Debug>(claim: Result<Claim<'_, V>, Refusal>) -> ClaimId {
    match claim {
        Ok(Claim::Acquired(id)) => id,
        other => panic!("expected an acquired claim, got {other:?}"),
    }
}

/// Deterministic generator for the randomized runs.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }

    pub fn pick(&mut self, bound: usize) -> usize {
        usize::try_from(self.next(bound as u64)).unwrap_or(0)
    }
}

/// Put `entry` in `store` whatever the store already holds: through `insert`
/// when it is accepted, else by overwriting a spare record in place, which is
/// how a second record under one key or claim id gets past a backend.
pub fn plant<S: Storage<Key = u64>>(store: &mut S, entry: Entry<u64, S::Value>)
where
    S::Value: Clone,
{
    if store.insert(entry.clone()).is_some() {
        return;
    }
    let spare = Entry::new(u64::MAX, None, None, ClaimId::from_raw(0), at(1), None);
    let slot = store
        .insert(spare)
        .unwrap_or_else(|| panic!("no room to plant"));
    let Some(held) = store.get_mut(slot) else {
        panic!("the spare record was just stored");
    };
    *held = entry;
}
