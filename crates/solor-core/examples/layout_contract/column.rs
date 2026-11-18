use super::layout::{
    LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Origin, Query,
    Result, Size,
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
}
