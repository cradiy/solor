//! Host-paced transitions for retained element styles.
use std::{collections::HashMap, hash::Hash, time::Duration};

use crate::{
    Edges, NodeId, Pixels, Rgba,
    layout::{ComputedLength, Dimension, FlexFactor},
    px,
    style::Corners,
};

/// Element style properties with interpolated display values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TransitionProperty {
    /// Preferred width. Changes to or from automatic sizing apply immediately.
    Width,
    /// Preferred height. Changes to or from automatic sizing apply immediately.
    Height,
    /// Lower bound on the element's width.
    MinWidth,
    /// Upper bound on the element's width.
    MaxWidth,
    /// Lower bound on the element's height.
    MinHeight,
    /// Upper bound on the element's height.
    MaxHeight,
    /// Initial size on the flex main axis, before free space is distributed.
    FlexBasis,
    /// Relative share of surplus space on the flex main axis.
    FlexGrow,
    /// Flex shrink factor, weighted by the item's inner base size when space is scarce.
    FlexShrink,
    /// Spacing between adjacent children along a flex container's main axis.
    Gap,
    /// Space between each border edge and the element content.
    Padding,
    /// The four radii of the border box.
    BorderRadius,
    /// Opacity of the composited subtree.
    Opacity,
    /// Solid background color or position along a background color palette.
    BackgroundColor,
    /// Color of the outline surrounding the element.
    OutlineColor,
    /// Thickness of the outline stroke.
    OutlineWidth,
    /// Distance from the border box to the outline's inner edge.
    OutlineOffset,
    /// Solid border colors on all four sides.
    BorderColor,
    /// Solid color of the top border.
    BorderTopColor,
    /// Solid color of the right border.
    BorderRightColor,
    /// Solid color of the bottom border.
    BorderBottomColor,
    /// Solid color of the left border.
    BorderLeftColor,
}

impl TransitionProperty {
    pub(crate) const OUTLINE_LENGTHS: [Self; 2] = [Self::OutlineWidth, Self::OutlineOffset];

    const BORDER_EDGES: [Self; 4] = [
        Self::BorderTopColor,
        Self::BorderRightColor,
        Self::BorderBottomColor,
        Self::BorderLeftColor,
    ];

    pub(crate) const COLORS: [Self; 6] = {
        let [top, right, bottom, left] = Self::BORDER_EDGES;
        [
            Self::BackgroundColor,
            Self::OutlineColor,
            top,
            right,
            bottom,
            left,
        ]
    };

    pub(crate) fn border_edge(self) -> Option<usize> {
        Self::BORDER_EDGES.iter().position(|edge| *edge == self)
    }

    pub(crate) fn declare(self, rules: &mut crate::style::TransitionRules, timing: Transition) {
        if self == Self::BorderColor {
            for edge in Self::BORDER_EDGES {
                rules.remove(&edge);
            }
        }
        rules.insert(self, timing);
    }

    pub(crate) fn timing(self, rules: &crate::style::TransitionRules) -> Option<Transition> {
        rules
            .get(&self)
            .or_else(|| {
                self.border_edge()
                    .and_then(|_| rules.get(&Self::BorderColor))
            })
            .copied()
    }
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

struct Track<T, D> {
    data: D,
    timing: Transition,
    start: Option<Duration>,
    from: T,
    target: T,
}

#[derive(Default)]
pub(crate) struct Transitions {
    pub(crate) opacity: Tracks<f32>,
    pub(crate) dimensions: Tracks<ComputedLength, (NodeId, Dimension)>,
    pub(crate) flex_factors: Tracks<f32, (NodeId, FlexFactor)>,
    pub(crate) gaps: Tracks<Pixels>,
    pub(crate) padding: Tracks<Edges, NodeId, crate::styled_body::ApplyPadding>,
    pub(crate) radii: Tracks<Corners<Pixels>>,
    pub(crate) lengths: Tracks<Pixels, (NodeId, TransitionProperty)>,
    pub(crate) colors: Tracks<Rgba, (NodeId, TransitionProperty)>,
    pub(crate) palette_position: Tracks<f32>,
}

impl Transitions {
    pub(crate) fn is_active(&self) -> bool {
        !self.opacity.active.is_empty()
            || !self.dimensions.active.is_empty()
            || !self.flex_factors.active.is_empty()
            || !self.gaps.active.is_empty()
            || !self.padding.active.is_empty()
            || !self.radii.active.is_empty()
            || !self.lengths.active.is_empty()
            || !self.colors.active.is_empty()
            || !self.palette_position.active.is_empty()
    }

    pub(crate) fn remove(&mut self, id: NodeId) {
        self.opacity.remove(id);
        self.gaps.remove(id);
        self.padding.remove_with_data(id);
        for factor in FlexFactor::ALL {
            self.flex_factors.remove((id, factor));
        }
        self.radii.remove(id);
        for dimension in Dimension::ALL {
            self.dimensions.remove((id, dimension));
        }
        for property in TransitionProperty::COLORS {
            self.colors.remove((id, property));
        }
        for property in TransitionProperty::OUTLINE_LENGTHS {
            self.lengths.remove((id, property));
        }
        self.palette_position.remove(id);
    }
}

pub(crate) struct Tracks<T, K = NodeId, D = ()> {
    active: HashMap<K, Track<T, D>>,
    last_frame: Duration,
}

impl<T, K, D> Default for Tracks<T, K, D> {
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

impl Interpolate for Pixels {
    fn interpolate(self, target: Self, t: f32) -> Self {
        let start = f64::from(self);
        px((start + (f64::from(target) - start) * f64::from(t)) as f32)
    }
}

impl TransitionProperty {
    pub(crate) fn flex_factor(self) -> Option<FlexFactor> {
        match self {
            Self::FlexGrow => Some(FlexFactor::Grow),
            Self::FlexShrink => Some(FlexFactor::Shrink),
            _ => None,
        }
    }

    pub(crate) fn dimension(self) -> Option<Dimension> {
        Some(match self {
            Self::FlexBasis => Dimension::FlexBasis,
            Self::Width => Dimension::Width,
            Self::Height => Dimension::Height,
            Self::MinWidth => Dimension::MinWidth,
            Self::MaxWidth => Dimension::MaxWidth,
            Self::MinHeight => Dimension::MinHeight,
            Self::MaxHeight => Dimension::MaxHeight,
            _ => return None,
        })
    }
}

impl Interpolate for ComputedLength {
    fn interpolate(self, target: Self, t: f32) -> Self {
        match (self, target) {
            (Self::Pixels(a), Self::Pixels(b)) => Self::Pixels(a.interpolate(b, t)),
            (Self::Fraction(a), Self::Fraction(b)) => Self::Fraction(a.interpolate(b, t)),
            _ => {
                let (a, x) = self.components();
                let (b, y) = target.components();
                Self::Mixed(a.interpolate(b, t), x.interpolate(y, t))
            }
        }
    }
}

impl Interpolate for Edges {
    fn interpolate(self, target: Self, t: f32) -> Self {
        Self {
            top: self.top.interpolate(target.top, t),
            right: self.right.interpolate(target.right, t),
            bottom: self.bottom.interpolate(target.bottom, t),
            left: self.left.interpolate(target.left, t),
        }
    }
}

impl Interpolate for Corners<Pixels> {
    fn interpolate(self, target: Self, t: f32) -> Self {
        Self {
            top_left: self.top_left.interpolate(target.top_left, t),
            top_right: self.top_right.interpolate(target.top_right, t),
            bottom_right: self.bottom_right.interpolate(target.bottom_right, t),
            bottom_left: self.bottom_left.interpolate(target.bottom_left, t),
        }
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

impl<T: Interpolate, K: Copy + Eq + Hash> Tracks<T, K> {
    pub(crate) fn remove(&mut self, key: K) -> Option<T> {
        self.remove_with_data(key).map(|(target, ())| target)
    }

    pub(crate) fn retarget(
        &mut self,
        key: K,
        current: T,
        target: T,
        timing: Option<Transition>,
    ) -> T {
        self.retarget_with_data(key, current, target, timing, ())
    }

    pub(crate) fn advance(
        &mut self,
        now: Duration,
        mut apply: impl FnMut(K, T) -> crate::Result<()>,
    ) -> crate::Result<()> {
        self.advance_with_data(now, |key, value, ()| apply(key, value))
    }
}

impl<T: Interpolate, K: Copy + Eq + Hash, D: Copy> Tracks<T, K, D> {
    pub(crate) fn remove_with_data(&mut self, key: K) -> Option<(T, D)> {
        self.active
            .remove(&key)
            .map(|track| (track.target, track.data))
    }

    pub(crate) fn retarget_with_data(
        &mut self,
        key: K,
        current: T,
        target: T,
        timing: Option<Transition>,
        data: D,
    ) -> T {
        let Some(timing) = timing.filter(|t| !t.duration.is_zero() && current != target) else {
            self.remove_with_data(key);
            return target;
        };
        let previous = self.active.get(&key);
        if previous.is_some_and(|track| track.target == target && track.timing == timing) {
            return current;
        }
        // A retargeted track starts at the time of the value currently on screen.
        let start = previous.and_then(|track| track.start.map(|_| self.last_frame));
        self.active.insert(
            key,
            Track {
                data,
                timing,
                start,
                from: current,
                target,
            },
        );
        current
    }

    pub(crate) fn advance_with_data(
        &mut self,
        now: Duration,
        mut apply: impl FnMut(K, T, D) -> crate::Result<()>,
    ) -> crate::Result<()> {
        self.last_frame = now;
        let mut result = Ok(());
        self.active.retain(|&key, track| {
            if result.is_err() {
                return true;
            }
            let elapsed = now.saturating_sub(*track.start.get_or_insert(now));
            let finished = elapsed >= track.timing.duration;
            let value = if finished {
                track.target
            } else if elapsed.is_zero() {
                track.from
            } else {
                let t = (elapsed.as_secs_f64() / track.timing.duration.as_secs_f64()) as f32;
                let t = match track.timing.easing {
                    Easing::Linear => t,
                    Easing::EaseOut => t * (2.0 - t),
                };
                track.from.interpolate(track.target, t)
            };
            result = apply(key, value, track.data);
            !finished || result.is_err()
        });
        result
    }
}
