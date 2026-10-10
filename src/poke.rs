use crate::attest::epoch_of;
use crate::matcher::Match;
use crate::model::Handle;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
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
    // Wall-clock day (seconds since epoch / 86400) the sent counts belong to.
    // Rolls over automatically on the first poke of a new day.
    day: u64,
    sent_today: HashMap<Handle, u32>,
    // Latest decline per pair, keyed with the handles sorted so (a, b) and
    // (b, a) are the same entry.
    declines: HashMap<(Handle, Handle), Decline>,
}

/// On-disk shape. Explicit rather than derived: JSON object keys must be
/// strings, so the pair-keyed decline map is stored as a list.
#[derive(Serialize, Deserialize)]
struct PokerState {
    day: u64,
    sent: HashMap<Handle, u32>,
    declines: Vec<DeclineEntry>,
}

#[derive(Serialize, Deserialize)]
struct DeclineEntry {
    a: Handle,
    b: Handle,
    at_secs: u64,
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
        self.roll_day(now);
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
        self.roll_day(now);
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

    fn roll_day(&mut self, now: SystemTime) {
        let d = epoch_of(now);
        if d != self.day {
            self.day = d;
            self.sent_today.clear();
        }
    }

    /// Persist rate-limit and decline state. Written atomically (temp file +
    /// rename) so a crash mid-write cannot leave half a file.
    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let st = PokerState {
            day: self.day,
            sent: self.sent_today.clone(),
            declines: self
                .declines
                .iter()
                .map(|((a, b), d)| DeclineEntry {
                    a: a.clone(),
                    b: b.clone(),
                    at_secs: d.at.duration_since(SystemTime::UNIX_EPOCH).map(|x| x.as_secs()).unwrap_or(0),
                })
                .collect(),
        };
        let text = serde_json::to_string(&st)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = path.as_ref().with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Load persisted state. A missing file is a fresh start; a corrupt file
    /// fails closed rather than silently resetting someone's rate limits.
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Poker> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Poker::default()),
            Err(e) => return Err(e),
        };
        let st: PokerState = serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Poker {
            day: st.day,
            sent_today: st.sent,
            declines: st
                .declines
                .into_iter()
                .map(|e| (pair(&e.a, &e.b), Decline { at: SystemTime::UNIX_EPOCH + Duration::from_secs(e.at_secs) }))
                .collect(),
        })
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
    fn state_roundtrips_through_disk() {
        let path = std::env::temp_dir().join(format!("pokkecon-poker-test-{}.json", std::process::id()));
        let mut p = Poker::default();
        let m = a_match("ann", "bo");
        p.poke_at(&m, t0());
        p.decline(&m, &Handle("bo".into()), t0());
        p.save(&path).unwrap();

        let q = Poker::load(&path).unwrap();
        // rate limit carried over: one of the day's three pokes is spent
        assert_eq!(q.sent_today.get(&Handle("ann".into())), Some(&1));
        // decline carried over
        assert!(q.on_cooldown(&Handle("ann".into()), &Handle("bo".into()), t0()));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn corrupt_state_fails_closed_and_missing_file_is_fresh() {
        let path = std::env::temp_dir().join(format!("pokkecon-poker-corrupt-{}.json", std::process::id()));
        std::fs::write(&path, "not json").unwrap();
        assert!(Poker::load(&path).is_err());
        std::fs::remove_file(&path).ok();
        let missing = std::env::temp_dir().join(format!("pokkecon-poker-missing-{}.json", std::process::id()));
        assert!(Poker::load(&missing).is_ok());
    }

    #[test]
    fn day_rollover_clears_rate_limits_but_keeps_declines() {
        let mut p = Poker::default();
        let m = a_match("ann", "bo");
        p.decline(&m, &Handle("bo".into()), t0());
        for _ in 0..MAX_POKES_PER_DAY {
            p.sent_today.insert(Handle("cy".into()), MAX_POKES_PER_DAY);
        }
        let next_day = t0() + Duration::from_secs(60 * 60 * 24);
        // cy is capped today; after rollover the cap is gone
        let m2 = a_match("cy", "dee");
        assert!(p.poke_at(&m2, t0()).is_empty());
        assert_eq!(p.poke_at(&m2, next_day).len(), 2);
        // ann x bo declined exactly one cooldown ago: it expires on the same boundary
        assert!(!p.on_cooldown(&Handle("ann".into()), &Handle("bo".into()), next_day));
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
