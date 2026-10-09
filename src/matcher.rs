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
