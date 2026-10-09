use crate::attest::Receipt;
use crate::model::Handle;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

const HALF_LIFE: Duration = Duration::from_secs(60 * 60 * 24 * 30);

/// Standing = sum of attested acts, each decayed by age. Not spendable, not displayed.
#[derive(Default)]
pub struct Standing {
    acts: HashMap<Handle, Vec<SystemTime>>,
}

impl Standing {
    /// Only the giver gains standing. Receivers are not charged.
    pub fn record(&mut self, r: &Receipt) {
        self.acts.entry(r.giver.clone()).or_default().push(r.at);
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
