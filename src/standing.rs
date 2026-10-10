use crate::attest::{epoch_of, valid, Identity, Receipt};
use crate::model::Handle;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

const HALF_LIFE: Duration = Duration::from_secs(60 * 60 * 24 * 30);
/// Receipts older than this are refused, so a stale receipt cannot be replayed later.
pub const MAX_RECEIPT_AGE: Duration = Duration::from_secs(60 * 60 * 24 * 7);
/// Allowed clock skew for receipts dated slightly in the future.
pub const MAX_FUTURE_SKEW: Duration = Duration::from_secs(60 * 5);
/// Cap on counted acts per (giver, receiver) pair inside one window. Blunts two-phone farming.
pub const PAIR_CAP: usize = 2;
pub const PAIR_WINDOW: Duration = Duration::from_secs(60 * 60 * 24 * 7);

#[derive(Debug, PartialEq, Eq)]
pub enum Reject {
    Replay,
    TooOld,
    FromTheFuture,
    PairCap,
}

/// Standing = sum of attested acts, each decayed by age. Not spendable, not displayed.
/// `record` assumes the caller already checked `attest::valid`; it adds replay,
/// age and per-pair limits on top.
#[derive(Default)]
pub struct Standing {
    acts: HashMap<Handle, Vec<SystemTime>>,
    seen: HashSet<Vec<u8>>,
    pairs: HashMap<(Handle, Handle), Vec<SystemTime>>,
}

impl Standing {
    /// Only the giver gains standing. Receivers are not charged.
    pub fn record(&mut self, r: &Receipt, now: SystemTime) -> Result<(), Reject> {
        if r.at > now + MAX_FUTURE_SKEW {
            return Err(Reject::FromTheFuture);
        }
        if now.duration_since(r.at).unwrap_or_default() > MAX_RECEIPT_AGE {
            return Err(Reject::TooOld);
        }
        let id = r.id();
        if self.seen.contains(&id) {
            return Err(Reject::Replay);
        }
        let pair = self.pairs.entry((r.giver.clone(), r.receiver.clone())).or_default();
        pair.retain(|t| now.duration_since(*t).unwrap_or_default() <= PAIR_WINDOW);
        if pair.len() >= PAIR_CAP {
            return Err(Reject::PairCap);
        }
        pair.push(r.at);
        self.seen.insert(id);
        self.acts.entry(r.giver.clone()).or_default().push(r.at);
        Ok(())
    }

    /// Rebuild standing from the device's own receipt log. Entries were fresh
    /// when the device wrote them, so the age and future-skew checks for NEW
    /// receipts do not apply here; signature validity, replay and the pair cap
    /// still do. Receipts that fail are skipped, not fatal: a log line is
    /// local evidence, and time alone (decay, pair windows) changes what counts.
    pub fn from_receipts(receipts: &[Receipt], now: SystemTime) -> Standing {
        let mut s = Standing::default();
        for r in receipts {
            if !valid(r) || s.seen.contains(&r.id()) {
                continue;
            }
            let pair = s.pairs.entry((r.giver.clone(), r.receiver.clone())).or_default();
            pair.retain(|t| r.at.duration_since(*t).unwrap_or_default() <= PAIR_WINDOW);
            if pair.len() >= PAIR_CAP {
                continue;
            }
            pair.push(r.at);
            s.seen.insert(r.id());
            s.acts.entry(r.giver.clone()).or_default().push(r.at);
        }
        s
    }

    /// The local device's own standing across handle rotations. Only the
    /// holder of the master secret can link its past handles to itself; this
    /// map never leaves the device. Everyone else's standing still resets at
    /// each rotation, which is the accepted privacy tradeoff (see README).
    pub fn own_score(&self, id: &Identity, now: SystemTime) -> f64 {
        self.acts
            .iter()
            .filter(|(h, times)| {
                times.iter().any(|t| {
                    let e = epoch_of(*t);
                    // signing and dating can straddle an epoch boundary
                    (e.saturating_sub(1)..=e + 1).any(|x| id.handle(x) == **h)
                })
            })
            .map(|(_, times)| {
                times
                    .iter()
                    .map(|t| {
                        let age = now.duration_since(*t).unwrap_or_default().as_secs_f64();
                        0.5f64.powf(age / HALF_LIFE.as_secs_f64())
                    })
                    .sum::<f64>()
            })
            .sum()
    }

    pub fn score(&self, who: &Handle, now: SystemTime) -> f64 {
        self.acts
            .get(who)
            .map(|v| {
                v.iter()
                    .map(|t| {
                        let age = now.duration_since(*t).unwrap_or_default().as_secs_f64();
                        0.5f64.powf(age / HALF_LIFE.as_secs_f64())
                    })
                    .sum()
            })
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attest::tests::{id, signed};

    fn h(s: &str) -> Handle {
        id(match s { "ann" => 1, "bo" => 2, _ => 3 }).handle(0)
    }
    fn rc(g: &str, r: &str, at: SystemTime, nonce: u64) -> Receipt {
        let n = |s: &str| id(match s { "ann" => 1, "bo" => 2, _ => 3 });
        signed(&n(g), &n(r), at, nonce)
    }
    fn t0() -> SystemTime { SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000) }

    #[test]
    fn replay_is_rejected_and_counts_once() {
        let mut s = Standing::default();
        let r = rc("ann", "bo", t0(), 1);
        assert_eq!(s.record(&r, t0()), Ok(()));
        assert_eq!(s.record(&r, t0()), Err(Reject::Replay));
        assert!((s.score(&h("ann"), t0()) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pair_cap_limits_farming_then_resets() {
        let mut s = Standing::default();
        for n in 0..PAIR_CAP as u64 {
            assert_eq!(s.record(&rc("ann", "bo", t0(), n), t0()), Ok(()));
        }
        assert_eq!(s.record(&rc("ann", "bo", t0(), 99), t0()), Err(Reject::PairCap));
        // a different counterparty is unaffected
        assert_eq!(s.record(&rc("ann", "cy", t0(), 5), t0()), Ok(()));
        // after the window the pair may count again
        let later = t0() + PAIR_WINDOW + Duration::from_secs(1);
        assert_eq!(s.record(&rc("ann", "bo", later, 100), later), Ok(()));
    }

    #[test]
    fn stale_and_future_receipts_are_rejected() {
        let mut s = Standing::default();
        let now = t0() + MAX_RECEIPT_AGE + Duration::from_secs(1);
        assert_eq!(s.record(&rc("ann", "bo", t0(), 1), now), Err(Reject::TooOld));
        assert_eq!(s.record(&rc("ann", "bo", now + Duration::from_secs(3600), 2), now), Err(Reject::FromTheFuture));
    }

    #[test]
    fn from_receipts_rebuilds_and_skips_bad_entries() {
        let good = rc("ann", "bo", t0(), 1);
        let mut tampered = rc("ann", "bo", t0(), 2);
        tampered.nonce = 99; // breaks both signatures
        let s = Standing::from_receipts(&[good.clone(), tampered, good], t0()); // dup is a replay
        assert!((s.score(&h("ann"), t0()) - 1.0).abs() < 1e-9);
        // age rules for new receipts do not apply to the device's own log
        let later = t0() + MAX_RECEIPT_AGE + Duration::from_secs(1);
        let old = rc("ann", "bo", t0(), 7);
        let s2 = Standing::from_receipts(&[old], later);
        assert!(s2.score(&h("ann"), later) > 0.0);
    }

    #[test]
    fn own_score_links_own_handles_across_epochs() {
        use crate::attest::tests::id;
        let ann = id(1);
        // ann's acts in epochs 0 and 1, bo's act in epoch 0
        let a0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let e1 = SystemTime::UNIX_EPOCH + Duration::from_secs(crate::attest::EPOCH_SECS + 1000);
        let r0 = rc("ann", "bo", a0, 1);
        // signed with epoch-1 handles and keys
        let mut r1 = Receipt::new(id(1).handle(1), id(2).handle(1), crate::model::Category::Carry, 1, 2, e1, 2);
        r1.sign_as_giver(&id(1), 1);
        r1.sign_as_receiver(&id(2), 1);
        let rb = rc("bo", "ann", a0, 3);
        let s = Standing::from_receipts(&[r0, r1, rb], e1);
        let mine = s.own_score(&ann, e1);
        let e0_only = s.score(&ann.handle(0), e1);
        let e1_only = s.score(&ann.handle(1), e1);
        assert!((mine - (e0_only + e1_only)).abs() < 1e-9);
        assert!(mine > e0_only);
        // bo cannot link ann's handles; his own_score sees only his own act
        assert!((s.own_score(&id(2), e1) - s.score(&id(2).handle(0), e1)).abs() < 1e-9);
    }

    #[test]
    fn score_decays_by_half_at_half_life() {
        let mut s = Standing::default();
        s.record(&rc("ann", "bo", t0(), 1), t0()).unwrap();
        assert!((s.score(&h("ann"), t0() + HALF_LIFE) - 0.5).abs() < 1e-9);
    }
}
