//! Helpers shared by the crate's test modules.

use core::time::Duration;

use crate::{Claim, ClaimId};

pub fn at(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

pub fn acquired<V>(claim: &Claim<'_, V>) -> ClaimId {
    let Claim::Acquired(id) = claim else {
        panic!("expected acquired claim");
    };
    *id
}
