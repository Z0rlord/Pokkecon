use crate::matcher::Match;
use crate::model::Handle;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

pub const MAX_POKES_PER_DAY: u32 = 3;

/// After either side declines, the same pair is not poked again for this long,
/// even if their signals still match.
pub const DECLINE_COOLDOWN: Duration = Duration::from_secs(60 * 60 * 24);

#[derive(Debug)]
pub struct Poke {
    pub to: Handle,
    pub text: String,
}

/// A recipient said no to a match. Declines only gate future pokes for the
/// pair; they never reach standing, so saying no costs nothing.
#[derive(Clone, Debug)]
pub struct Decline {
    pub at: SystemTime,
}

#[derive(Default)]
pub struct Poker {
    sent_today: HashMap<Handle, u32>,
    // Latest decline per pair, keyed with the handles sorted so (a, b) and
    // (b, a) are the same entry.
    declines: HashMap<(Handle, Handle), Decline>,
}

fn pair(a: &Handle, b: &Handle) -> (Handle, Handle) {
    if a <= b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    }
}

impl Poker {
    /// Like `poke_at`, with the current time. Convenience for the live path;
    /// tests use `poke_at` for deterministic time.
    pub fn poke(&mut self, m: &Match) -> Vec<Poke> {
        self.poke_at(m, SystemTime::now())
    }

    /// Turn a match into one poke per side, unless either side is rate limited
    /// or the pair is inside a decline cooldown. Returns nothing if the match
    /// must not fire, so no one is poked for a dead match.
    pub fn poke_at(&mut self, m: &Match, now: SystemTime) -> Vec<Poke> {
        let (a, b) = (&m.offer.who, &m.need.who);
        if self.on_cooldown(a, b, now) {
            return vec![];
        }
        let cap = |h: &Handle, s: &HashMap<Handle, u32>| *s.get(h).unwrap_or(&0) >= MAX_POKES_PER_DAY;
        if cap(a, &self.sent_today) || cap(b, &self.sent_today) {
            return vec![];
        }
        *self.sent_today.entry(a.clone()).or_insert(0) += 1;
        *self.sent_today.entry(b.clone()).or_insert(0) += 1;
        vec![
            Poke { to: a.clone(), text: format!("Someone nearby could use help: {:?}. Up for it?", m.need.category) },
            Poke { to: b.clone(), text: format!("Someone nearby may help with: {:?}. Heads up.", m.offer.category) },
        ]
    }

    /// Record that `who` declined the match. Returns false if `who` is not
    /// part of the match. Only the pair and the time are kept, never who
    /// said no, and nothing is written to standing.
    pub fn decline(&mut self, m: &Match, who: &Handle, now: SystemTime) -> bool {
        if *who != m.offer.who && *who != m.need.who {
            return false;
        }
        self.declines.insert(pair(&m.offer.who, &m.need.who), Decline { at: now });
        true
    }

    /// True while the pair's latest decline is still inside its cooldown.
    /// A decline dated in the future counts as active, fail closed.
    pub fn on_cooldown(&self, a: &Handle, b: &Handle, now: SystemTime) -> bool {
        match self.declines.get(&pair(a, b)) {
            Some(d) => now.duration_since(d.at).map(|e| e < DECLINE_COOLDOWN).unwrap_or(true),
            None => false,
        }
    }

    pub fn new_day(&mut self) {
        self.sent_today.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::standing::Standing;

    fn sig(id: u64, who: &str, kind: Kind) -> Signal {
        Signal {
            id,
            who: Handle(who.into()),
            kind,
            category: Category::Carry,
            cell: Cell("a".into()),
            expires: SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000),
        }
    }

    fn a_match(offer: &str, need: &str) -> Match {
        Match { offer: sig(1, offer, Kind::Offer), need: sig(2, need, Kind::Need) }
    }

    fn t0() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)
    }

    #[test]
    fn decline_blocks_re_poke_within_cooldown() {
        let mut p = Poker::default();
        let m = a_match("ann", "bo");
        assert_eq!(p.poke_at(&m, t0()).len(), 2);
        p.decline(&m, &Handle("bo".into()), t0());
        assert!(p.on_cooldown(&Handle("ann".into()), &Handle("bo".into()), t0()));
        assert!(p.poke_at(&m, t0() + Duration::from_secs(60)).is_empty());
    }

    #[test]
    fn cooldown_is_per_pair() {
        let mut p = Poker::default();
        let m1 = a_match("ann", "bo");
        p.decline(&m1, &Handle("bo".into()), t0());
        // A different pairing for ann is not affected.
        let m2 = a_match("ann", "cy");
        assert_eq!(p.poke_at(&m2, t0()).len(), 2);
    }

    #[test]
    fn poke_allowed_again_after_cooldown() {
        let mut p = Poker::default();
        let m = a_match("ann", "bo");
        p.decline(&m, &Handle("ann".into()), t0());
        assert!(p.poke_at(&m, t0() + DECLINE_COOLDOWN - Duration::from_secs(1)).is_empty());
        assert_eq!(p.poke_at(&m, t0() + DECLINE_COOLDOWN + Duration::from_secs(1)).len(), 2);
    }

    #[test]
    fn future_dated_decline_counts_as_active() {
        let mut p = Poker::default();
        let m = a_match("ann", "bo");
        p.decline(&m, &Handle("bo".into()), t0() + Duration::from_secs(60));
        assert!(p.poke_at(&m, t0()).is_empty());
    }

    #[test]
    fn decline_by_outsider_is_ignored() {
        let mut p = Poker::default();
        let m = a_match("ann", "bo");
        assert!(!p.decline(&m, &Handle("cy".into()), t0()));
        assert_eq!(p.poke_at(&m, t0()).len(), 2);
    }

    #[test]
    fn decline_leaves_standing_untouched() {
        let mut p = Poker::default();
        let s = Standing::default();
        let m = a_match("ann", "bo");
        let before = s.score(&Handle("bo".into()), t0());
        p.decline(&m, &Handle("bo".into()), t0());
        assert_eq!(s.score(&Handle("bo".into()), t0()), before);
    }
}
