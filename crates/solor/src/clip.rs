//! Logical clip bounds shared by frame recording and pointer routing.
use std::{ops::Range, sync::Arc};

pub(crate) use crate::layout::OverflowClip;
use crate::{
    Pixels, Result,
    layout::{Origin, Size},
    style::{LayoutProperties, Overflow},
};
use anyhow::Context as _;

/// Effective overflow bounds and rounded ancestors in drawing coordinates.
/// Each specified range includes its start and excludes its end.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentClip {
    /// Horizontal interval, or unrestricted when absent.
    pub x: Option<Range<Pixels>>,
    /// Vertical interval, or unrestricted when absent.
    pub y: Option<Range<Pixels>>,
    /// Rounded viewports intersected with the axis bounds.
    pub rounded: Option<Arc<RoundedClips>>,
}

/// A rounded clipping box in the drawing coordinate system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedClip {
    /// Upper-left corner of the clip.
    pub origin: Origin,
    /// Dimensions before intersecting ancestor clips.
    pub size: Size,
    /// Outer radii fitted to the box.
    pub radii: crate::style::Corners<Pixels>,
}

/// One rounded viewport and its shared ancestor clips.
#[derive(Debug, PartialEq)]
pub struct RoundedClips {
    shape: RoundedClip,
    parent: Option<Arc<Self>>,
}

impl RoundedClips {
    /// Iterates from the innermost rounded viewport to the outermost.
    pub fn iter(&self) -> impl Iterator<Item = &RoundedClip> {
        std::iter::successors(Some(self), |node| node.parent.as_deref()).map(|node| &node.shape)
    }
}

impl RoundedClip {
    fn contains(self, point: Origin) -> bool {
        let x = f64::from(point.x) - f64::from(self.origin.x);
        let y = f64::from(point.y) - f64::from(self.origin.y);
        let w = f64::from(self.size.width);
        let h = f64::from(self.size.height);
        if x < 0. || y < 0. || x >= w || y >= h {
            return false;
        }
        let r = self.radii;
        [
            (x, y, r.top_left),
            (w - x, y, r.top_right),
            (w - x, h - y, r.bottom_right),
            (x, h - y, r.bottom_left),
        ]
        .into_iter()
        .all(|(dx, dy, radius)| {
            let r = f64::from(radius);
            dx >= r || dy >= r || (dx - r).powi(2) + (dy - r).powi(2) <= r * r
        })
    }
}

impl ContentClip {
    pub(crate) fn intersect_rect(mut self, origin: Origin, size: Size) -> Result<Self> {
        anyhow::ensure!(
            size.width.is_extent() && size.height.is_extent(),
            "invalid clip extent"
        );
        intersect_axis(&mut self.x, origin.x, size.width)?;
        intersect_axis(&mut self.y, origin.y, size.height)?;
        Ok(self)
    }

    pub(crate) fn intersects(&self, origin: Origin, size: Size) -> bool {
        fn overlaps(range: Option<&Range<Pixels>>, start: Pixels, length: Pixels) -> bool {
            length.get() > 0.0
                && range.is_none_or(|r| {
                    r.start < r.end
                        && start < r.end
                        && f64::from(start) + f64::from(length) > f64::from(r.start)
                })
        }
        overlaps(self.x.as_ref(), origin.x, size.width)
            && overlaps(self.y.as_ref(), origin.y, size.height)
    }
}

impl OverflowClip {
    pub(crate) fn from_layout(layout: Option<&LayoutProperties>, gutter: Size) -> Self {
        let clips = |policy| {
            matches!(
                policy,
                Some(Overflow::Hidden | Overflow::Scroll | Overflow::Auto)
            )
        };
        Self {
            x: clips(layout.and_then(|v| v.overflow_x)) || gutter.width > crate::px(0.),
            y: clips(layout.and_then(|v| v.overflow_y)) || gutter.height > crate::px(0.),
        }
    }

    pub(crate) fn resolve(
        &self,
        inherited: Option<Arc<ContentClip>>,
        origin: Origin,
        viewport: Size,
        size: Size,
        radii: crate::style::Corners<Pixels>,
    ) -> Result<NodeClips> {
        let content = if self.x || self.y {
            let mut clip = inherited.as_deref().cloned().unwrap_or_default();
            if self.x {
                intersect_axis(&mut clip.x, origin.x, viewport.width)?;
            }
            if self.y {
                intersect_axis(&mut clip.y, origin.y, viewport.height)?;
            }
            if self.x && self.y && radii != Default::default() {
                clip.rounded = Some(Arc::new(RoundedClips {
                    shape: RoundedClip {
                        origin,
                        size,
                        radii: radii.fit_to(size),
                    },
                    parent: clip.rounded,
                }));
            }
            Some(Arc::new(clip))
        } else {
            inherited.clone()
        };
        Ok(NodeClips { inherited, content })
    }
}

fn intersect_axis(axis: &mut Option<Range<Pixels>>, start: Pixels, length: Pixels) -> Result<()> {
    let end = start
        .checked_add(length)
        .context("clip edge outside layout range")?;
    *axis = Some(match axis.as_ref() {
        Some(old) => old.start.max(start)..old.end.min(end),
        None => start..end,
    });
    Ok(())
}

#[derive(Default)]
pub(crate) struct NodeClips {
    pub(crate) inherited: Option<Arc<ContentClip>>,
    pub(crate) content: Option<Arc<ContentClip>>,
}

impl NodeClips {
    pub(crate) fn contains(&self, point: Origin) -> bool {
        let within_parent = self.inherited.as_ref().is_none_or(|clip| {
            clip.x.as_ref().is_none_or(|x| x.contains(&point.x))
                && clip.y.as_ref().is_none_or(|y| y.contains(&point.y))
        });
        within_parent
            && self
                .content
                .as_ref()
                .and_then(|clip| clip.rounded.as_deref())
                .is_none_or(|shapes| shapes.iter().all(|shape| shape.contains(point)))
    }
}
