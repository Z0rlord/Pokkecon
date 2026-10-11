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

/// An act vouched by someone with no standing still counts this much,
/// so legitimate new users can get started (cold start).
pub const BASE_VOUCH: f64 = 0.25;
/// Extra weight from the attester's own standing, capped so a high-standing
/// voucher is worth at most BASE_VOUCH + VOUCH_BONUS_CAP per act.
pub const VOUCH_BONUS_CAP: f64 = 0.75;

#[derive(Debug, PartialEq, Eq)]
pub enum Reject {
    Replay,
    TooOld,
    FromTheFuture,
    PairCap,
}

/// Standing = sum of attested acts, each weighted by the attester's standing
/// at the time and decayed by age. Not spendable, not displayed.
/// `record` assumes the caller already checked `attest::valid`; it adds replay,
/// age and per-pair limits on top.
#[derive(Default)]
pub struct Standing {
    acts: HashMap<Handle, Vec<(SystemTime, f64)>>,
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
        // weight at the act's time, so live recording and replay agree exactly
        if !self.count(r, r.at) {
            return Err(if self.seen.contains(&r.id()) { Reject::Replay } else { Reject::PairCap });
        }
        Ok(())
    }

    /// Shared by `record` and `from_receipts`: replay check, per-pair cap,
    /// then count the act weighted by the receiver's standing at `at`.
    fn count(&mut self, r: &Receipt, at: SystemTime) -> bool {
        if self.seen.contains(&r.id()) {
            return false;
        }
        let w = self.weight(&r.receiver, at);
        let pair = self.pairs.entry((r.giver.clone(), r.receiver.clone())).or_default();
        pair.retain(|t| r.at.duration_since(*t).unwrap_or_default() <= PAIR_WINDOW);
        if pair.len() >= PAIR_CAP {
            return false;
        }
        pair.push(r.at);
        self.seen.insert(r.id());
        self.acts.entry(r.giver.clone()).or_default().push((r.at, w));
        true
    }

    /// The weight of one vouch: a base for cold start, plus the attester's
    /// own standing, capped. A collusion ring of fresh phones can only ever
    /// vouch at base weight; the cap stops one whale dominating.
    fn weight(&self, attester: &Handle, at: SystemTime) -> f64 {
        BASE_VOUCH + self.score(attester, at).min(VOUCH_BONUS_CAP)
    }

    /// Rebuild standing from the device's own receipt log. Entries were fresh
    /// when the device wrote them, so the age and future-skew checks for NEW
    /// receipts do not apply here; signature validity, replay and the pair cap
    /// still do. Receipts are replayed oldest first (ties broken by receipt
    /// id) so attestation weights see the same history the live path saw.
    /// Receipts that fail are skipped, not fatal: a log line is local
    /// evidence, and time alone (decay, pair windows) changes what counts.
    pub fn from_receipts(receipts: &[Receipt], now: SystemTime) -> Standing {
        let mut rs: Vec<&Receipt> = receipts.iter().filter(|r| valid(r)).collect();
        rs.sort_by_key(|r| (r.at, r.id()));
        let mut s = Standing::default();
        for r in rs {
            s.count(r, r.at);
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
                times.iter().any(|(t, _)| {
                    let e = epoch_of(*t);
                    // signing and dating can straddle an epoch boundary
                    (e.saturating_sub(1)..=e + 1).any(|x| id.handle(x) == **h)
                })
            })
            .map(|(_, times)| times.iter().map(|(t, w)| w * decay(*t, now)).sum::<f64>())
            .sum()
    }

    pub fn score(&self, who: &Handle, now: SystemTime) -> f64 {
        self.acts
            .get(who)
            .map(|v| v.iter().map(|(t, w)| w * decay(*t, now)).sum())
            .unwrap_or(0.0)
    }
}

fn decay(t: SystemTime, now: SystemTime) -> f64 {
    let age = now.duration_since(t).unwrap_or_default().as_secs_f64();
    0.5f64.powf(age / HALF_LIFE.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attest::tests::{id, signed};
    use crate::model::Category;

    fn h(s: &str) -> Handle {
        id(match s { "ann" => 1, "bo" => 2, _ => 3 }).handle(0)
    }
    fn n(s: &str) -> Identity {
        id(match s { "ann" => 1, "bo" => 2, _ => 3 })
    }
    fn rc(g: &str, r: &str, at: SystemTime, nonce: u64) -> Receipt {
        signed(&n(g), &n(r), at, nonce)
    }
    fn t0() -> SystemTime { SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000) }

    #[test]
    fn replay_is_rejected_and_counts_once() {
        let mut s = Standing::default();
        let r = rc("ann", "bo", t0(), 1);
        assert_eq!(s.record(&r, t0()), Ok(()));
        assert_eq!(s.record(&r, t0()), Err(Reject::Replay));
        // bo has no standing, so the vouch lands at base weight
        assert!((s.score(&h("ann"), t0()) - BASE_VOUCH).abs() < 1e-9);
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
        assert!((s.score(&h("ann"), t0()) - BASE_VOUCH).abs() < 1e-9);
        // age rules for new receipts do not apply to the device's own log
        let later = t0() + MAX_RECEIPT_AGE + Duration::from_secs(1);
        let old = rc("ann", "bo", t0(), 7);
        let s2 = Standing::from_receipts(&[old], later);
        assert!(s2.score(&h("ann"), later) > 0.0);
    }

    #[test]
    fn replay_matches_live_recording() {
        // a small history, recorded live in order
        let rs = vec![
            rc("ann", "bo", t0(), 1),
            rc("bo", "ann", t0() + Duration::from_secs(10), 2),
            rc("ann", "cy", t0() + Duration::from_secs(20), 3),
            rc("cy", "ann", t0() + Duration::from_secs(30), 4),
        ];
        let mut live = Standing::default();
        for r in &rs {
            live.record(r, r.at + MAX_FUTURE_SKEW).ok();
        }
        // rebuilt from a shuffled log (log order is not time order)
        let mut shuffled = rs.clone();
        shuffled.reverse();
        let rebuilt = Standing::from_receipts(&shuffled, t0() + Duration::from_secs(30));
        let end = t0() + Duration::from_secs(60);
        for who in ["ann", "bo", "cy"] {
            assert!((live.score(&h(who), end) - rebuilt.score(&h(who), end)).abs() < 1e-9, "{}", who);
        }
    }

    #[test]
    fn vouch_from_established_attester_counts_more() {
        let mut s = Standing::default();
        // cy earns standing by giving to ann (vouched at base)
        s.record(&rc("cy", "ann", t0(), 1), t0()).unwrap();
        // ann gives to cy (cy attests, cy has standing) vs ann gives to dee (dee has none)
        s.record(&rc("ann", "cy", t0(), 2), t0()).unwrap();
        s.record(&rc("ann", "dee", t0(), 3), t0()).unwrap();
        let ann_score = s.score(&h("ann"), t0());
        let base_only = 2.0 * BASE_VOUCH;
        assert!(ann_score > base_only);
        // cy's vouch was worth base + min(cy's score, cap)
        let expected = BASE_VOUCH + (BASE_VOUCH + BASE_VOUCH.min(VOUCH_BONUS_CAP)) + BASE_VOUCH;
        assert!((ann_score - expected).abs() < 1e-9);
    }

    #[test]
    fn fresh_collusion_ring_grows_slowly() {
        // a fresh ring's first mutual vouch is worth less than one vouch
        // from an established attester (base + full bonus = 1.0)
        let mut s = Standing::default();
        s.record(&rc("ann", "bo", t0(), 1), t0()).unwrap();
        s.record(&rc("bo", "ann", t0(), 2), t0()).unwrap();
        let first_round = s.score(&h("ann"), t0()) + s.score(&h("bo"), t0());
        assert!(first_round < BASE_VOUCH + VOUCH_BONUS_CAP);
        // and the pair cap bounds a whole window of two-phone farming
        for n in 1..PAIR_CAP as u64 {
            s.record(&rc("ann", "bo", t0(), 10 + n), t0()).unwrap();
            s.record(&rc("bo", "ann", t0(), 110 + n), t0()).unwrap();
        }
        let max_per_window = PAIR_CAP as f64 * (BASE_VOUCH + VOUCH_BONUS_CAP);
        assert!(s.score(&h("ann"), t0()) < max_per_window);
        assert!(s.score(&h("bo"), t0()) < max_per_window);
    }

    #[test]
    fn own_score_links_own_handles_across_epochs() {
        let ann = id(1);
        // ann's acts in epochs 0 and 1, bo's act in epoch 0
        let a0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let e1 = SystemTime::UNIX_EPOCH + Duration::from_secs(crate::attest::EPOCH_SECS + 1000);
        let r0 = rc("ann", "bo", a0, 1);
        // signed with epoch-1 handles and keys
        let mut r1 = Receipt::new(id(1).handle(1), id(2).handle(1), Category::Carry, 1, 2, e1, 2);
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
        assert!((s.score(&h("ann"), t0() + HALF_LIFE) - 0.5 * BASE_VOUCH).abs() < 1e-9);
    }
}
