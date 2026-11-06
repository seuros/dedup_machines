use alloc::string::{String, ToString};
use core::time::Duration;

use super::*;
use crate::{Attempt, Claim, ClaimId};

fn at(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}
fn acquired<V>(claim: Claim<'_, V>) -> ClaimId {
    let Claim::Acquired(id) = claim else {
        panic!("expected acquired claim");
    };
    id
}

#[test]
fn owned_keys_follow_the_fixed_lifecycle() {
    let mut ledger = HashDedup::<String, String>::with_capacity(1);
    let id = acquired(ledger.claim("a".into(), at(0), at(10)));
    assert_eq!(ledger.claim("a".into(), at(1), at(10)), Claim::InProgress);
    assert_eq!(ledger.claim("b".into(), at(1), at(10)), Claim::Full);
    assert_eq!(ledger.complete_with(id, "id-1".into(), at(2)), Ok(()));
    assert_eq!(
        ledger.complete_with(id, "id-2".into(), at(2)),
        Err("id-2".to_string())
    );
    assert_eq!(
        ledger.claim("a".into(), at(9), at(1)),
        Claim::Completed(&"id-1".into())
    );
    let replacement = acquired(ledger.claim("a".into(), at(10), at(5)));
    assert!(!ledger.complete(id, at(10)));
    assert!(ledger.complete(replacement, at(10)));
}

#[test]
fn freed_slots_are_reused_without_resurrecting_handles() {
    let mut ledger = HashDedup::<u64>::with_capacity(2);
    let first = acquired(ledger.claim(1, at(0), at(10)));
    let second = acquired(ledger.claim(2, at(0), at(10)));
    assert!(ledger.abandon(first));
    let third = acquired(ledger.claim(3, at(0), at(10)));
    assert_eq!(ledger.claim(4, at(0), at(10)), Claim::Full);
    assert!(!ledger.abandon(first));
    assert!(ledger.complete(third, at(1)));
    assert!(ledger.complete(second, at(1)));
    assert_eq!(ledger.claim(1, at(1), at(10)), Claim::Full);
    assert_eq!(ledger.claim(3, at(1), at(10)), Claim::Completed(&()));
    assert!(matches!(
        ledger.claim(1, at(10), at(10)),
        Claim::Acquired(_)
    ));
    assert_eq!(ledger.store.slots.len(), 2);
}

#[test]
fn fingerprints_conflict_on_owned_keys() {
    let mut ledger = HashDedup::<String, u8>::with_capacity(4);
    let id = acquired(ledger.claim_with("turn".into(), 1, at(0), at(10)));
    assert_eq!(
        ledger.claim_with("turn".into(), 2, at(0), at(10)),
        Claim::Conflict
    );
    assert_eq!(ledger.complete_with(id, 200, at(1)), Ok(()));
    assert_eq!(
        ledger.claim_with("turn".into(), 2, at(1), at(10)),
        Claim::Conflict
    );
    assert_eq!(
        ledger.claim_with("turn".into(), 1, at(1), at(10)),
        Claim::Completed(&200)
    );
}

#[test]
fn partition_caps_and_owner_expiry_apply_to_owned_keys() {
    let mut ledger = HashDedup::<(String, String)>::with_capacity(8).with_partition_cap(1);
    let session = |name: &str| Attempt::new((name.into(), "src".into())).partition(1);
    let doomed = acquired(ledger.admit(session("s1"), at(0), at(10)));
    assert_eq!(
        ledger.admit(session("s2"), at(0), at(10)),
        Claim::PartitionFull
    );
    assert!(matches!(
        ledger.admit(
            Attempt::new(("s2".into(), "src".into())).partition(2),
            at(0),
            at(10)
        ),
        Claim::Acquired(_)
    ));
    assert_eq!(ledger.expire_where(|key| key.0 == "s1"), 1);
    assert!(!ledger.abandon(doomed));
    assert!(matches!(
        ledger.admit(session("s3"), at(1), at(10)),
        Claim::Acquired(_)
    ));
    assert_eq!(
        ledger.claim(("s2".into(), "src".into()), at(1), at(10)),
        Claim::InProgress
    );
}

#[test]
fn zero_capacity_and_invalid_time_fail_closed() {
    let mut ledger = HashDedup::<u64>::with_capacity(0);
    assert_eq!(ledger.claim(0, at(0), at(1)), Claim::Full);
    assert_eq!(ledger.claim(0, at(1), Duration::ZERO), Claim::Rejected);
    assert_eq!(ledger.claim(0, at(0), at(1)), Claim::Rejected);
}
