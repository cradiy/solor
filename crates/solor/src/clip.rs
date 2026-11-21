//! Logical clip bounds shared by frame recording and pointer routing.
use std::{ops::Range, sync::Arc};

use crate::{
    Pixels, Result,
    layout::{Origin, Size},
    style::{LayoutProperties, Overflow},
};
use anyhow::Context as _;

/// A rectangular clip in the command's coordinate system, with independent axes.
/// Each specified range includes its start and excludes its end.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentClip {
    /// Horizontal interval, or unrestricted when absent.
    pub x: Option<Range<Pixels>>,
    /// Vertical interval, or unrestricted when absent.
    pub y: Option<Range<Pixels>>,
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

    pub(crate) fn contains(&self, point: Origin) -> bool {
        self.x.as_ref().is_none_or(|x| x.contains(&point.x))
            && self.y.as_ref().is_none_or(|y| y.contains(&point.y))
    }
}

#[derive(PartialEq, Eq)]
pub(crate) struct OverflowClip {
    x: bool,
    y: bool,
}

impl OverflowClip {
    pub(crate) fn from_layout(layout: Option<&LayoutProperties>) -> Self {
        let clips = |policy| {
            matches!(
                policy,
                Some(Overflow::Hidden | Overflow::Scroll | Overflow::Auto)
            )
        };
        Self {
            x: clips(layout.and_then(|v| v.overflow_x)),
            y: clips(layout.and_then(|v| v.overflow_y)),
        }
    }

    pub(crate) fn resolve(
        &self,
        inherited: Option<Arc<ContentClip>>,
        origin: Origin,
        size: Size,
    ) -> Result<NodeClips> {
        let content = if self.x || self.y {
            let mut clip = inherited.as_deref().cloned().unwrap_or_default();
            if self.x {
                intersect_axis(&mut clip.x, origin.x, size.width)?;
            }
            if self.y {
                intersect_axis(&mut clip.y, origin.y, size.height)?;
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

pub(crate) struct NodeClips {
    pub(crate) inherited: Option<Arc<ContentClip>>,
    pub(crate) content: Option<Arc<ContentClip>>,
}
