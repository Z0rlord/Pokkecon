use std::time::{Duration, SystemTime};

/// Rotating pseudonym. The router never learns a real identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(pub String);

/// Coarse geo cell id (e.g. an H3 cell at ~1 km). Never raw coordinates.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Cell(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Carry,
    Lend,
    Guide,
    Meal,
    Repair,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Offer,
    Need,
}

#[derive(Clone, Debug)]
pub struct Signal {
    pub id: u64,
    pub who: Handle,
    pub kind: Kind,
    pub category: Category,
    pub cell: Cell,
    pub expires: SystemTime,
}

impl Signal {
    pub fn live(&self, now: SystemTime) -> bool {
        now < self.expires
    }
}

pub const SIGNAL_TTL: Duration = Duration::from_secs(60 * 60);
