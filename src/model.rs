use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// Rotating pseudonym. The router never learns a real identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Handle(pub String);

/// Coarse geo cell id (e.g. an H3 cell at ~1 km). Never raw coordinates.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cell(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    Carry,
    Lend,
    Guide,
    Meal,
    Repair,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Offer,
    Need,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

impl Category {
    pub fn from_name(s: &str) -> Option<Category> {
        match s {
            "carry" => Some(Category::Carry),
            "lend" => Some(Category::Lend),
            "guide" => Some(Category::Guide),
            "meal" => Some(Category::Meal),
            "repair" => Some(Category::Repair),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Category::Carry => "carry",
            Category::Lend => "lend",
            Category::Guide => "guide",
            Category::Meal => "meal",
            Category::Repair => "repair",
        }
    }
}

pub const SIGNAL_TTL: Duration = Duration::from_secs(60 * 60);
