use std::any::Any;

use solor_core::tree::NodeId;

use super::fixtures::{Drawing, Wrapping};
use super::layout::{
    LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query,
    Result, Scene, Size,
};

fn add(a: u32, b: u32) -> Result<u32> {
    a.checked_add(b).ok_or(LayoutError::InvalidSize)
}

pub struct Button {
    pub units: u32,
    pub advance: u32,
    pub line_height: u32,
    pub ascent: u32,
    pub padding: u32,
}

impl Button {
    fn natural(&self) -> Result<Size> {
        if self.advance == 0 || self.line_height == 0 || self.ascent > self.line_height {
            return Err(LayoutError::InvalidSize);
        }
        let inset = add(self.padding, self.padding)?;
        Ok(Size {
            width: add(
                self.units
                    .checked_mul(self.advance)
                    .ok_or(LayoutError::InvalidSize)?,
                inset,
            )?,
            height: add(self.line_height, inset)?,
        })
    }
}

impl LayoutNode for Button {
    fn measure(&mut self, _cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let natural = self.natural()?;
        Ok(Metrics {
            size: Size {
                width: query.resolve(natural.width),
                height: natural.height,
            },
            baseline: Some(add(self.padding, self.ascent)?),
        })
    }

    fn layout(&mut self, _cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        let natural = self.natural()?;
        let y = add(
            self.padding,
            assigned.height.saturating_sub(natural.height) / 2,
        )?;
        Ok(LayoutOutput {
            baseline: Some(add(y, self.ascent)?),
            content: Size {
                width: assigned.width.max(natural.width),
                height: assigned.height.max(natural.height),
            },
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub struct Row {
    pub padding: u32,
    pub gap: u32,
}

struct Plan {
    width: u32,
    children: [Metrics; 3],
    height: u32,
    baseline: u32,
}

fn band(heights: [u32; 3], baselines: [Option<u32>; 3]) -> Result<(u32, u32)> {
    let baseline = baselines[1].unwrap_or(0).max(baselines[2].unwrap_or(0));
    let descent = heights[1]
        .saturating_sub(baselines[1].unwrap_or(0))
        .max(heights[2].saturating_sub(baselines[2].unwrap_or(0)));
    Ok((heights[0].max(add(baseline, descent)?), baseline))
}

impl Row {
    fn plan(
        &self,
        count: usize,
        query: Query,
        mut measure: impl FnMut(usize, Query) -> Result<Metrics>,
    ) -> Result<Plan> {
        if count != 3 {
            return Err(LayoutError::UnexpectedChildCount);
        }
        let mut children = [
            measure(0, Query::Natural)?,
            measure(1, Query::Natural)?,
            measure(2, Query::Natural)?,
        ];
        let fixed = add(
            add(add(self.padding, self.padding)?, add(self.gap, self.gap)?)?,
            add(children[0].size.width, children[2].size.width)?,
        )?;
        let width = query.resolve(add(fixed, children[1].size.width)?);
        children[1] = measure(1, Query::Exact(width.saturating_sub(fixed)))?;
        let (height, baseline) = band(
            children.map(|child| child.size.height),
            children.map(|child| child.baseline),
        )?;
        Ok(Plan {
            width,
            children,
            height,
            baseline,
        })
    }
}

impl LayoutNode for Row {
    fn measure(&mut self, cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        let plan = self.plan(cx.child_count(), query, |index, query| {
            cx.measure_child(index, query)
        })?;
        Ok(Metrics {
            size: Size {
                width: plan.width,
                height: add(plan.height, add(self.padding, self.padding)?)?,
            },
            baseline: Some(add(self.padding, plan.baseline)?),
        })
    }

    fn layout(&mut self, cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        let plan = self.plan(
            cx.child_count(),
            Query::Exact(assigned.width),
            |index, query| cx.measure_child(index, query),
        )?;
        let outputs = [
            cx.layout_child(0, plan.children[0].size)?,
            cx.layout_child(1, plan.children[1].size)?,
            cx.layout_child(2, plan.children[2].size)?,
        ];
        let heights = std::array::from_fn(|index| {
            plan.children[index]
                .size
                .height
                .max(outputs[index].content.height)
        });
        let (height, baseline) = band(heights, outputs.map(|output| output.baseline))?;
        let inset = add(self.padding, self.padding)?;
        let top = add(
            self.padding,
            assigned.height.saturating_sub(add(height, inset)?) / 2,
        )?;
        let mut x = self.padding;
        let mut content = Size::default();
        for index in 0..3 {
            let offset = if index == 0 || outputs[index].baseline.is_none() {
                height.saturating_sub(heights[index]) / 2
            } else {
                baseline - outputs[index].baseline.unwrap()
            };
            let y = add(top, offset)?;
            cx.place_child(index, Origin { x, y })?;
            content.width = content.width.max(add(
                x,
                plan.children[index]
                    .size
                    .width
                    .max(outputs[index].content.width),
            )?);
            content.height = content.height.max(add(y, heights[index])?);
            x = add(x, plan.children[index].size.width)?;
            if index < 2 {
                x = add(x, self.gap)?;
            }
        }
        content.width = add(content.width, self.padding)?;
        content.height = add(content.height, self.padding)?;
        Ok(LayoutOutput {
            baseline: Some(add(top, baseline)?),
            content,
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub struct Ids {
    pub label: NodeId,
    pub button: NodeId,
}

pub fn fixture() -> (Scene, Ids) {
    let mut scene = Scene::new(Row { padding: 2, gap: 3 });
    let root = scene.root();
    scene.append(root, Drawing::new(10, 10)).unwrap();
    let label = scene
        .append(
            root,
            Wrapping {
                units: 20,
                advance: 2,
                line_height: 5,
                ascent: 4,
            },
        )
        .unwrap();
    let button = scene
        .append(
            root,
            Button {
                units: 4,
                advance: 2,
                line_height: 6,
                ascent: 4,
                padding: 2,
            },
        )
        .unwrap();
    (scene, Ids { label, button })
}

pub fn run() {
    let (mut scene, ids) = fixture();
    let (mut reference, other) = fixture();
    for (width, units) in [(80, 4), (50, 4), (50, 8), (20, 8), (0, 8), (80, 4)] {
        scene
            .edit_layout::<Button>(ids.button, |button| button.units = units)
            .unwrap();
        reference
            .edit_layout::<Button>(other.button, |button| button.units = units)
            .unwrap();
        scene.run(Query::Exact(width), None, false).unwrap();
        reference.run(Query::Exact(width), None, true).unwrap();
        assert_eq!(scene.snapshot(), reference.snapshot());
    }
    scene
        .edit_layout::<Wrapping>(ids.label, |label| label.units = 30)
        .unwrap();
    reference
        .edit_layout::<Wrapping>(other.label, |label| label.units = 30)
        .unwrap();
    let before = scene.counts();
    scene
        .run(
            Query::Natural,
            Some(Size {
                width: 50,
                height: 40,
            }),
            false,
        )
        .unwrap();
    reference
        .run(
            Query::Natural,
            Some(Size {
                width: 50,
                height: 40,
            }),
            true,
        )
        .unwrap();
    assert_eq!(scene.snapshot(), reference.snapshot());
    let after = scene.counts();
    println!(
        "row negotiation: {} queries, {} cache hits, {} measurement bodies, {} layout bodies",
        after.queries - before.queries,
        after.cache_hits - before.cache_hits,
        after.measures - before.measures,
        after.layouts - before.layouts
    );
}
