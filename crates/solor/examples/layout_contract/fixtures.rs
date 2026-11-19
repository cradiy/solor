use solor::px;
use solor::tree::NodeId;

use super::layout::{
    LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Query, Result,
    Scene, Size,
};

pub use super::column::Column;

pub struct Wrapping {
    pub units: u32,
    pub advance: u32,
    pub line_height: u32,
    pub ascent: u32,
}

impl Wrapping {
    fn compute(&self, query: Query) -> Result<(Metrics, Size)> {
        if self.advance == 0 || self.line_height == 0 || self.ascent > self.line_height {
            return Err(LayoutError::InvalidSize);
        }
        let natural = self
            .units
            .checked_mul(self.advance)
            .ok_or(LayoutError::InvalidSize)?;
        let width = query.resolve(px(natural as f32));
        let columns = ((width.get() / self.advance as f32).floor() as u32).max(1);
        let lines = self.units.div_ceil(columns);
        let height = lines
            .checked_mul(self.line_height)
            .ok_or(LayoutError::InvalidSize)?;
        let content = Size {
            width: px(self
                .units
                .min(columns)
                .checked_mul(self.advance)
                .ok_or(LayoutError::InvalidSize)? as f32),
            height: px(height as f32),
        };
        Ok((
            Metrics {
                size: Size {
                    width,
                    height: px(height as f32),
                },
                baseline: (lines > 0).then_some(px(self.ascent as f32)),
            },
            content,
        ))
    }
}

impl LayoutNode for Wrapping {
    fn measure(&mut self, _cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        Ok(self.compute(query)?.0)
    }

    fn layout(&mut self, _cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        let (metrics, content) = self.compute(Query::Exact(assigned.width))?;
        Ok(LayoutOutput {
            baseline: metrics.baseline,
            content,
        })
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
            size: Size {
                width: px(width as f32),
                height: px(height as f32),
            },
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
                Fault::Error => return Err(LayoutError::Node("injected")),
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
                width: query.resolve(self.size.width),
                height: self.size.height,
            },
            baseline: None,
        })
    }

    fn layout(&mut self, _cx: &mut LayoutContext<'_, '_>, _assigned: Size) -> Result<LayoutOutput> {
        #[cfg(test)]
        self.check(Phase::Layout)?;
        Ok(LayoutOutput {
            baseline: None,
            content: self.size,
        })
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
    let mut scene = Scene::new(Column::new(px(4.0), px(2.0)));
    let root = scene.root();
    let left = scene.append(root, Column::new(px(2.0), px(3.0))).unwrap();
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
    let right = scene.append(root, Column::new(px(2.0), px(3.0))).unwrap();
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
