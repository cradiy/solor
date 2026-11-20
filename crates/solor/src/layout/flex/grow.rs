//! Distribute surplus line space among items with positive growth factors.
use super::{Pixels, px};

#[derive(Default)]
pub(super) struct Growth {
    unlimited_weight: f64,
    caps: Vec<Cap>,
}

struct Cap {
    space: f64,
    weight: f64,
    threshold: f64,
    remaining_weight: f64,
}

impl Growth {
    pub(super) fn push(&mut self, weight: f32, base: Pixels, limit: Option<Pixels>) {
        if weight == 0.0 {
            return;
        }
        let weight = f64::from(weight);
        if let Some(limit) = limit {
            let space = (f64::from(limit) - f64::from(base)).max(0.0);
            if space > 0.0 {
                self.caps.push(Cap {
                    space,
                    weight,
                    threshold: space / weight,
                    remaining_weight: 0.0,
                });
            }
        } else {
            self.unlimited_weight += weight;
        }
    }

    // An item freezes when its share per unit of weight reaches its threshold.
    // Sum the still-active weights from the far end so a large frozen factor
    // cannot cancel the remaining smaller factors through subtraction.
    pub(super) fn resolve(mut self, free: Pixels) -> (f64, Pixels) {
        if free <= px(0.0) {
            return (0.0, free);
        }
        self.caps
            .sort_unstable_by(|a, b| a.threshold.total_cmp(&b.threshold));
        let mut weight = self.unlimited_weight;
        for cap in self.caps.iter_mut().rev() {
            weight += cap.weight;
            cap.remaining_weight = weight;
        }
        let initial = f64::from(free);
        let mut remaining = initial;
        for cap in self.caps {
            let unit = (remaining / cap.remaining_weight).min(initial);
            if unit <= cap.threshold {
                return (
                    unit,
                    px((remaining - unit * cap.remaining_weight).max(0.0) as f32),
                );
            }
            remaining = (remaining - cap.space).max(0.0);
        }
        let unit = if self.unlimited_weight > 0.0 {
            (remaining / self.unlimited_weight).min(initial)
        } else {
            // All growing items hit their maxima.
            initial
        };
        (
            unit,
            px((remaining - unit * self.unlimited_weight).max(0.0) as f32),
        )
    }
}
