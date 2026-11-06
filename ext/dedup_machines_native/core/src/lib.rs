#![no_std]
#![doc = include_str!("../README.md")]

use core::time::Duration;

/// Opaque generation-checked claim, scoped to one ledger lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimId {
    slot: usize,
    generation: u64,
}

/// Claim outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Claim {
    /// Caller owns a new pending claim.
    Acquired(ClaimId),
    /// Another live attempt owns this key.
    InProgress,
    /// This key completed within its retention interval.
    Completed,
    /// All slots contain unexpired records.
    Full,
    /// Invalid clock/TTL or exhausted generation space.
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry<K> {
    key: K,
    generation: u64,
    expires: Duration,
    completed: bool,
}

/// Fixed-capacity exact-key ledger.
#[derive(Debug, PartialEq, Eq)]
pub struct Dedup<K, const N: usize> {
    entries: [Option<Entry<K>>; N],
    generation: u64,
    observed: Duration,
}

impl<K: Copy + Eq, const N: usize> Default for Dedup<K, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Copy + Eq, const N: usize> Dedup<K, N> {
    /// Start an empty ledger with a fresh handle namespace.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: [None; N],
            generation: 0,
            observed: Duration::ZERO,
        }
    }

    /// Claim a key without evicting any unexpired entry.
    #[must_use]
    pub fn claim(&mut self, key: K, now: Duration, ttl: Duration) -> Claim {
        if !self.observe(now) || ttl.is_zero() {
            return Claim::Rejected;
        }
        let Some(expires) = now.checked_add(ttl) else {
            return Claim::Rejected;
        };
        for entry in &mut self.entries {
            if entry.is_some_and(|record| now >= record.expires) {
                *entry = None;
            }
        }
        if let Some(entry) = self.entries.iter().flatten().find(|entry| entry.key == key) {
            return if entry.completed {
                Claim::Completed
            } else {
                Claim::InProgress
            };
        }
        let Some(slot) = self.entries.iter().position(Option::is_none) else {
            return Claim::Full;
        };
        let Some(generation) = self.generation.checked_add(1) else {
            return Claim::Rejected;
        };
        self.generation = generation;
        self.entries[slot] = Some(Entry {
            key,
            generation,
            expires,
            completed: false,
        });
        Claim::Acquired(ClaimId { slot, generation })
    }

    /// Complete a live pending claim. Repeated or stale completions return `false`.
    #[must_use]
    pub fn complete(&mut self, claim: ClaimId, now: Duration) -> bool {
        if !self.observe(now) {
            return false;
        }
        let Some(Some(entry)) = self.entries.get_mut(claim.slot) else {
            return false;
        };
        if entry.generation != claim.generation || entry.completed || now >= entry.expires {
            return false;
        }
        entry.completed = true;
        true
    }

    /// Free a matching pending claim after a known failure, not an uncertain effect.
    #[must_use]
    pub fn abandon(&mut self, claim: ClaimId) -> bool {
        let Some(slot) = self.entries.get_mut(claim.slot) else {
            return false;
        };
        if !slot.is_some_and(|entry| entry.generation == claim.generation && !entry.completed) {
            return false;
        }
        *slot = None;
        true
    }

    fn observe(&mut self, now: Duration) -> bool {
        if now < self.observed {
            return false;
        }
        self.observed = now;
        true
    }
}

#[cfg(test)]
mod tests;
