use crate::model::*;
use crate::standing::Standing;
use std::time::SystemTime;

#[derive(Debug)]
pub struct Match {
    pub offer: Signal,
    pub need: Signal,
}

/// Pair each live Need with the best live Offer of the same category in the same cell.
/// Best = highest standing of the offerer, so reliable givers are poked first.
/// A person is never matched with themselves. Each signal is used at most once.
pub fn run(signals: &[Signal], standing: &Standing, now: SystemTime) -> Vec<Match> {
    let mut offers: Vec<&Signal> = signals
        .iter()
        .filter(|s| s.kind == Kind::Offer && s.live(now))
        .collect();
    offers.sort_by(|a, b| {
        standing
            .score(&b.who, now)
            .partial_cmp(&standing.score(&a.who, now))
            .unwrap()
    });

    let mut used: Vec<u64> = Vec::new();
    let mut out = Vec::new();
    for need in signals.iter().filter(|s| s.kind == Kind::Need && s.live(now)) {
        let pick = offers.iter().find(|o| {
            !used.contains(&o.id)
                && o.category == need.category
                && o.cell == need.cell
                && o.who != need.who
        });
        if let Some(o) = pick {
            used.push(o.id);
            out.push(Match { offer: (*o).clone(), need: need.clone() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sig(id: u64, who: &str, kind: Kind, cat: Category, cell: &str, exp: SystemTime) -> Signal {
        Signal { id, who: Handle(who.into()), kind, category: cat, cell: Cell(cell.into()), expires: exp }
    }
    fn now() -> SystemTime { SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000) }

    #[test]
    fn matches_same_cell_and_category_only() {
        let e = now() + SIGNAL_TTL;
        let s = vec![
            sig(1, "ann", Kind::Offer, Category::Carry, "a", e),
            sig(2, "bo", Kind::Need, Category::Carry, "b", e),
            sig(3, "cy", Kind::Need, Category::Meal, "a", e),
        ];
        assert!(run(&s, &Standing::default(), now()).is_empty());
    }

    #[test]
    fn never_matches_self_or_expired_and_uses_offer_once() {
        let e = now() + SIGNAL_TTL;
        let s = vec![
            sig(1, "ann", Kind::Offer, Category::Carry, "a", e),
            sig(2, "ann", Kind::Need, Category::Carry, "a", e),
            sig(3, "bo", Kind::Need, Category::Carry, "a", e),
            sig(4, "cy", Kind::Need, Category::Carry, "a", e),
            sig(5, "dee", Kind::Offer, Category::Carry, "a", now() - Duration::from_secs(1)),
        ];
        let m = run(&s, &Standing::default(), now());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].need.who, Handle("bo".into()));
    }
}
