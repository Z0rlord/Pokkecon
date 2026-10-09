use crate::attest::Receipt;
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
    fn score_decays_by_half_at_half_life() {
        let mut s = Standing::default();
        s.record(&rc("ann", "bo", t0(), 1), t0()).unwrap();
        assert!((s.score(&h("ann"), t0() + HALF_LIFE) - 0.5).abs() < 1e-9);
    }
}
