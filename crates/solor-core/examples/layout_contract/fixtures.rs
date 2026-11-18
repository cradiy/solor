use std::any::Any;

use solor_core::tree::NodeId;

use super::layout::{
    LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query,
    Result, Scene, Size,
};

fn add(a: u32, b: u32) -> Result<u32> {
    a.checked_add(b).ok_or(LayoutError::InvalidSize)
}

pub struct Column {
    pub padding: u32,
    pub gap: u32,
}

impl Column {
    fn child_query(&self, query: Query) -> Result<Query> {
        let inset = add(self.padding, self.padding)?;
        Ok(match query {
            Query::Natural => Query::Natural,
            Query::AtMost(width) | Query::Exact(width) => {
                Query::AtMost(width.saturating_sub(inset))
            }
        })
    }

    /// A stretched child receives the column's inner width. Content bounds
    /// still describe the measured children and their reported overflow.
    pub fn layout_rows(
        &self,
        cx: &mut LayoutContext<'_, '_>,
        assigned: Size,
        stretch: Option<usize>,
    ) -> Result<LayoutOutput> {
        let inset = add(self.padding, self.padding)?;
        let inner_width = assigned.width.saturating_sub(inset);
        let query = Query::AtMost(inner_width);
        let mut y = self.padding;
        let mut content = Size {
            width: inset,
            height: self.padding,
        };
        let mut baseline = None;
        for index in 0..cx.child_count() {
            if index > 0 {
                y = add(y, self.gap)?;
            }
            let child = cx.measure_child(index, query)?;
            let mut size = child.size;
            if stretch == Some(index) {
                size.width = inner_width;
            }
            let output = cx.layout_child(index, size)?;
            cx.place_child(index, Origin { x: self.padding, y })?;
            if index == 0 {
                baseline = output.baseline.map(|value| add(value, y)).transpose()?;
            }
            content.width = content.width.max(add(
                add(self.padding, child.size.width.max(output.content.width))?,
                self.padding,
            )?);
            content.height = content
                .height
                .max(add(y, child.size.height.max(output.content.height))?);
            y = add(y, child.size.height)?;
        }
        content.height = add(content.height.max(y), self.padding)?;
        Ok(LayoutOutput { baseline, content })
    }
}

impl LayoutNode for Column {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let child_query = self.child_query(query)?;
        let mut height = self.padding;
        let mut width = 0;
        let mut baseline = None;
        for index in 0..cx.child_count() {
            if index > 0 {
                height = add(height, self.gap)?;
            }
            let child = cx.measure_child(index, child_query)?;
            if index == 0 {
                baseline = child.baseline.map(|value| add(value, height)).transpose()?;
            }
            height = add(height, child.size.height)?;
            width = width.max(child.size.width);
        }
        Ok(Metrics {
            size: Size {
                width: query.resolve(add(width, add(self.padding, self.padding)?)?),
                height: add(height, self.padding)?,
            },
            baseline,
        })
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        self.layout_rows(cx, assigned, None)
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

impl Wrapping {
    fn compute(&self, query: Query) -> Result<(Metrics, Size)> {
        if self.advance == 0 || self.line_height == 0 || self.ascent > self.line_height {
            return Err(LayoutError::InvalidSize);
        }
        let natural = self
            .units
            .checked_mul(self.advance)
            .ok_or(LayoutError::InvalidSize)?;
        let width = query.resolve(natural);
        let columns = (width / self.advance).max(1);
        let lines = self.units.div_ceil(columns);
        let height = lines
            .checked_mul(self.line_height)
            .ok_or(LayoutError::InvalidSize)?;
        let content = Size {
            width: self
                .units
                .min(columns)
                .checked_mul(self.advance)
                .ok_or(LayoutError::InvalidSize)?,
            height,
        };
        Ok((
            Metrics {
                size: Size { width, height },
                baseline: (lines > 0).then_some(self.ascent),
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
