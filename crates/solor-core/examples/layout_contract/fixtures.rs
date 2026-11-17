use std::any::Any;

use solor_core::tree::NodeId;

use super::layout::{
    LayoutContext, LayoutError, LayoutNode, MeasureContext, Metrics, Origin, Query, Result, Scene,
    Size,
};

fn add(a: u32, b: u32) -> Result<u32> {
    a.checked_add(b).ok_or(LayoutError::InvalidSize)
}

pub struct Column {
    pub padding: u32,
    pub gap: u32,
}

impl Column {
    fn inner(&self, width: u32) -> Result<Query> {
        Query::new(
            width
                .checked_sub(add(self.padding, self.padding)?)
                .ok_or(LayoutError::InvalidSize)?,
        )
    }
}

impl LayoutNode for Column {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let inner = self.inner(query.width())?;
        let mut height = self.padding;
        let mut baseline = None;
        for index in 0..cx.child_count() {
            if index > 0 {
                height = add(height, self.gap)?;
            }
            let child = cx.measure_child(index, inner)?;
            if index == 0 {
                baseline = child.baseline.map(|value| add(value, height)).transpose()?;
            }
            height = add(height, child.size.height)?;
        }
        Ok(Metrics {
            size: Size {
                width: query.width(),
                height: add(height, self.padding)?,
            },
            baseline,
        })
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<()> {
        let inner = self.inner(assigned.width)?;
        let mut y = self.padding;
        for index in 0..cx.child_count() {
            if index > 0 {
                y = add(y, self.gap)?;
            }
            let child = cx.measure_child(index, inner)?;
            cx.place_child(index, child, Origin { x: self.padding, y })?;
            y = add(y, child.size.height)?;
        }
        if add(y, self.padding)? != assigned.height {
            return Err(LayoutError::InvalidSize);
        }
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub struct Wrapping {
    pub units: u32,
    pub advance: u32,
    pub line_height: u32,
    pub ascent: u32,
}

impl LayoutNode for Wrapping {
    fn measure(&mut self, _cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        if self.advance == 0 || self.line_height == 0 || self.ascent > self.line_height {
            return Err(LayoutError::InvalidSize);
        }
        let columns = (query.width() / self.advance).max(1);
        let lines = self.units.div_ceil(columns);
        let height = lines
            .checked_mul(self.line_height)
            .ok_or(LayoutError::InvalidSize)?;
        Ok(Metrics {
            size: Size {
                width: query.width(),
                height,
            },
            baseline: (lines > 0).then_some(self.ascent),
        })
    }

    fn layout(&mut self, _cx: &mut LayoutContext<'_, '_>, _assigned: Size) -> Result<()> {
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub struct Drawing {
    pub size: Size,
    pub ink: u32,
    #[cfg(test)]
    pub fault: Option<(Phase, Fault)>,
}

impl Drawing {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            size: Size { width, height },
            ink: 0,
            #[cfg(test)]
            fault: None,
        }
    }

    #[cfg(test)]
    fn check(&self, phase: Phase) -> Result<()> {
        if let Some((target, fault)) = self.fault
            && target == phase
        {
            match fault {
                Fault::Error => return Err(LayoutError::Injected),
                Fault::Panic => panic!("intentional layout fixture panic"),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Measure,
    Layout,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub enum Fault {
    Error,
    Panic,
}

impl LayoutNode for Drawing {
    fn measure(&mut self, _cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        #[cfg(test)]
        self.check(Phase::Measure)?;
        Ok(Metrics {
            size: Size {
                width: self.size.width.min(query.width()),
                height: self.size.height,
            },
            baseline: None,
        })
    }

    fn layout(&mut self, _cx: &mut LayoutContext<'_, '_>, _assigned: Size) -> Result<()> {
        #[cfg(test)]
        self.check(Phase::Layout)?;
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Scene {
    pub fn set_ink(&mut self, id: NodeId, ink: u32) -> Result<()> {
        self.body_mut::<Drawing>(id)?.ink = ink;
        Ok(())
    }
}

pub struct Ids {
    pub text: NodeId,
    pub drawing: NodeId,
    #[cfg(test)]
    pub left: NodeId,
    #[cfg(test)]
    pub right: NodeId,
}

pub fn nested() -> (Scene, Ids) {
    let mut scene = Scene::new(Column { padding: 4, gap: 2 });
    let root = scene.root();
    let left = scene.append(root, Column { padding: 2, gap: 3 }).unwrap();
    let text = scene
        .append(
            left,
            Wrapping {
                units: 8,
                advance: 5,
                line_height: 10,
                ascent: 7,
            },
        )
        .unwrap();
    let drawing = scene.append(left, Drawing::new(10, 4)).unwrap();
    let right = scene.append(root, Column { padding: 2, gap: 3 }).unwrap();
    scene
        .append(
            right,
            Wrapping {
                units: 20,
                advance: 5,
                line_height: 10,
                ascent: 7,
            },
        )
        .unwrap();
    scene.append(right, Drawing::new(12, 6)).unwrap();
    (
        scene,
        Ids {
            text,
            drawing,
            #[cfg(test)]
            left,
            #[cfg(test)]
            right,
        },
    )
}
