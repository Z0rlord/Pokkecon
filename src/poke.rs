use crate::matcher::Match;
use crate::model::Handle;
use std::collections::HashMap;

pub const MAX_POKES_PER_DAY: u32 = 3;

#[derive(Debug)]
pub struct Poke {
    pub to: Handle,
    pub text: String,
}

#[derive(Default)]
pub struct Poker {
    sent_today: HashMap<Handle, u32>,
}

impl Poker {
    /// Turn a match into one poke per side, unless either side is rate limited.
    /// Returns nothing if either side is capped, so no one is poked for a dead match.
    pub fn poke(&mut self, m: &Match) -> Vec<Poke> {
        let (a, b) = (&m.offer.who, &m.need.who);
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

    pub fn new_day(&mut self) {
        self.sent_today.clear();
    }
}
