//! Allocate a line deficit using the content sizes of participating items.
use super::{MeasuredItem, Pixels, px};

#[derive(Clone, Copy, Default)]
struct Weights {
    factors: f64,
    scaled: f64,
}

impl Weights {
    fn add(&mut self, other: Self) {
        self.factors += other.factors;
        self.scaled += other.scaled;
    }

    fn unit(self, initial: f64, remaining: f64) -> f64 {
        if self.scaled == 0.0 {
            return 0.0;
        }
        remaining.min(initial * self.factors) / self.scaled
    }
}

struct Limit {
    room: f64,
    threshold: f64,
    weights: Weights,
    following: Weights,
}

#[derive(Default)]
pub(super) struct Shrink {
    active: Weights,
    // Items that can surrender the same proportion per unit of weight freeze
    // together. Identical factors without binding minima need no allocation.
    first: Option<Limit>,
    others: Vec<Limit>,
}

impl Shrink {
    pub(super) fn push(&mut self, base: Pixels, item: &MeasuredItem) {
        let room = f64::from(base) - f64::from(minimum(item));
        if item.style.shrink == 0.0 || room <= 0.0 {
            return;
        }
        let weights = Weights {
            factors: f64::from(item.style.shrink),
            scaled: scaled_weight(item, base),
        };
        let threshold = room / weights.scaled;
        self.active.add(weights);
        if let Some(first) = &mut self.first
            && first.threshold == threshold
        {
            first.weights.add(weights);
            first.room += room;
            return;
        }
        let limit = Limit {
            room,
            threshold,
            weights,
            following: Weights::default(),
        };
        if self.first.is_none() {
            self.first = Some(limit);
        } else {
            self.others.push(limit);
        }
    }

    pub(super) fn resolve(mut self, deficit: Pixels) -> (ShrinkPlan, Pixels) {
        let initial = f64::from(deficit);
        let mut remaining = initial;
        let mut plan = ShrinkPlan {
            unit: self.active.unit(initial, remaining),
            frozen_through: 0.0,
        };
        if let Some(first) = self.first {
            if self.others.is_empty() {
                plan.unit = plan.unit.min(first.threshold);
            } else if first.threshold < plan.unit
                || self.others.iter().any(|v| v.threshold < plan.unit)
            {
                self.others.push(first);
                self.others
                    .sort_unstable_by(|a, b| a.threshold.total_cmp(&b.threshold));
                // Sum from the far end so freezing a large weight cannot
                // cancel the smaller weights that still participate.
                let mut following = Weights::default();
                for limit in self.others.iter_mut().rev() {
                    limit.following = following;
                    following.add(limit.weights);
                }
                let mut limits = self.others.as_slice();
                loop {
                    let count = limits.partition_point(|limit| limit.threshold < plan.unit);
                    let (batch, rest) = limits.split_at(count);
                    let Some(last) = batch.last() else { break };
                    for limit in batch {
                        remaining = (remaining - limit.room).max(0.0);
                    }
                    self.active = last.following;
                    plan.frozen_through = last.threshold;
                    plan.unit = self.active.unit(initial, remaining);
                    limits = rest;
                }
            }
        }
        let unused = (remaining - plan.unit * self.active.scaled).max(0.0);
        (plan, px(-unused as f32))
    }
}

fn minimum(item: &MeasuredItem) -> Pixels {
    item.main_limits.lower().max(item.shrink_inset)
}

fn scaled_weight(item: &MeasuredItem, base: Pixels) -> f64 {
    f64::from(item.style.shrink) * (f64::from(base) - f64::from(item.shrink_inset)).max(0.0)
}

#[derive(Clone, Copy)]
pub(super) struct ShrinkPlan {
    unit: f64,
    frozen_through: f64,
}

impl ShrinkPlan {
    pub(super) fn extent(&self, base: Pixels, item: &MeasuredItem) -> Pixels {
        let weight = scaled_weight(item, base);
        if weight == 0.0 {
            return base;
        }
        let floor = minimum(item).min(base);
        // Fractional factors can reduce the unit after other items freeze.
        if self.unit < self.frozen_through
            && (f64::from(base) - f64::from(floor)) / weight <= self.frozen_through
        {
            floor
        } else {
            px((f64::from(base) - self.unit * weight).max(f64::from(floor)) as f32)
        }
    }
}
