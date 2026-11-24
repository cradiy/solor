//! Host-paced transitions for retained element styles.
use std::{collections::HashMap, time::Duration};

use crate::{NodeId, Rgba};

/// Element style properties with interpolated display values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TransitionProperty {
    /// Opacity of the composited subtree.
    Opacity,
    /// Solid background color. Other fill types apply immediately.
    BackgroundColor,
}

impl AsRef<[TransitionProperty]> for TransitionProperty {
    fn as_ref(&self) -> &[Self] {
        std::slice::from_ref(self)
    }
}

/// Timing applied when the target value of an element property changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    duration: Duration,
    easing: Easing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Easing {
    Linear,
    EaseOut,
}

impl Transition {
    /// Creates a linear transition. A zero duration applies the target immediately.
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            easing: Easing::Linear,
        }
    }

    /// Starts quickly and decelerates quadratically towards the target.
    pub fn ease_out(mut self) -> Self {
        self.easing = Easing::EaseOut;
        self
    }
}

struct Track<T> {
    timing: Transition,
    start: Option<Duration>,
    from: T,
    target: T,
}

#[derive(Default)]
pub(crate) struct Transitions {
    pub(crate) opacity: Tracks<f32>,
    pub(crate) background: Tracks<Rgba>,
}

impl Transitions {
    pub(crate) fn is_active(&self) -> bool {
        !self.opacity.active.is_empty() || !self.background.active.is_empty()
    }

    pub(crate) fn remove(&mut self, id: NodeId) {
        self.opacity.remove(id);
        self.background.remove(id);
    }
}

pub(crate) struct Tracks<T> {
    active: HashMap<NodeId, Track<T>>,
    last_frame: Duration,
}

impl<T> Default for Tracks<T> {
    fn default() -> Self {
        Self {
            active: HashMap::new(),
            last_frame: Duration::ZERO,
        }
    }
}

pub(crate) trait Interpolate: Copy + PartialEq {
    fn interpolate(self, target: Self, progress: f32) -> Self;
}

impl Interpolate for f32 {
    fn interpolate(self, target: Self, t: f32) -> Self {
        self + (target - self) * t
    }
}

impl Interpolate for Rgba {
    fn interpolate(self, target: Self, t: f32) -> Self {
        let color =
            |c: Rgba| color::AlphaColor::<color::Srgb>::from_rgba8(c.red, c.green, c.blue, c.alpha);
        let mixed = color(self).lerp_rect(color(target), t).to_rgba8();
        Rgba::new(mixed.r, mixed.g, mixed.b, mixed.a)
    }
}

impl<T: Interpolate> Tracks<T> {
    pub(crate) fn remove(&mut self, id: NodeId) -> Option<T> {
        self.active.remove(&id).map(|track| track.target)
    }

    pub(crate) fn retarget(
        &mut self,
        id: NodeId,
        current: T,
        target: T,
        timing: Option<Transition>,
    ) -> T {
        let Some(timing) = timing.filter(|t| !t.duration.is_zero() && current != target) else {
            self.remove(id);
            return target;
        };
        let previous = self.active.get(&id);
        if previous.is_some_and(|track| track.target == target && track.timing == timing) {
            return current;
        }
        // A retargeted track starts at the time of the value currently on screen.
        let start = previous.and_then(|track| track.start.map(|_| self.last_frame));
        self.active.insert(
            id,
            Track {
                timing,
                start,
                from: current,
                target,
            },
        );
        current
    }

    pub(crate) fn advance(&mut self, now: Duration, mut apply: impl FnMut(NodeId, T)) {
        self.last_frame = now;
        self.active.retain(|&id, track| {
            let elapsed = now.saturating_sub(*track.start.get_or_insert(now));
            if elapsed >= track.timing.duration {
                apply(id, track.target);
                return false;
            }
            let t = (elapsed.as_secs_f64() / track.timing.duration.as_secs_f64()) as f32;
            let t = match track.timing.easing {
                Easing::Linear => t,
                Easing::EaseOut => t * (2.0 - t),
            };
            apply(
                id,
                if t == 0.0 {
                    track.from
                } else {
                    track.from.interpolate(track.target, t)
                },
            );
            true
        });
    }
}
