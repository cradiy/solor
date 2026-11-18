use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Shaping, Wrap, fontdb};

use crate::layout::{
    LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Query, Result,
    Size,
};

pub const FONT: &[u8] = include_bytes!("fonts/NotoSansCJKsc-fixture.otf");
pub const FAMILY: &str = "Noto Sans CJK SC";
pub const CACHE_SLOTS: usize = 3;
pub const MAX_TEXT_BYTES: usize = 4096;
const MAX_WIDTH: u32 = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub size: u16,
    pub line_height: u16,
}

impl Style {
    fn validate(self) -> Result<()> {
        if self.size == 0
            || self.size > 128
            || self.line_height < self.size
            || self.line_height > 256
        {
            return Err(LayoutError::InvalidSize);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    pub start: usize,
    pub end: usize,
    pub id: u16,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub source_line: usize,
    pub top: f32,
    pub baseline: f32,
    pub height: f32,
    pub width: f32,
    pub glyphs: Vec<Glyph>,
}

/// Owned immutable layout data; no borrowed Buffer or FontSystem state.
#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    pub text: String,
    pub style: Style,
    pub lines: Vec<Line>,
    pub content: Size,
    pub baseline: Option<u32>,
}

impl Paragraph {
    pub fn metrics(&self, query: Query) -> Metrics {
        Metrics {
            size: Size {
                width: query.resolve(self.content.width),
                height: self.content.height,
            },
            baseline: self.baseline,
        }
    }

    // Payload capacity only: excludes allocator rounding, Rc headers and engine internals.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.text.capacity()
            + self.lines.capacity() * std::mem::size_of::<Line>()
            + self
                .lines
                .iter()
                .map(|l| l.glyphs.capacity() * std::mem::size_of::<Glyph>())
                .sum::<usize>()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub requests: usize,
    pub hits: usize,
    pub shapes: usize,
}

pub struct Engine {
    fonts: FontSystem,
    face: fontdb::ID,
    pub counts: Counts,
}

pub type SharedEngine = Rc<RefCell<Engine>>;

fn extent(value: f32) -> Result<u32> {
    if !value.is_finite() || !(0.0..=(1 << 24) as f32).contains(&value) {
        return Err(LayoutError::InvalidSize);
    }
    Ok(value.ceil() as u32)
}

impl Engine {
    pub fn new() -> SharedEngine {
        let mut db = fontdb::Database::new();
        db.load_font_data(FONT.to_vec());
        assert_eq!(
            db.faces().count(),
            1,
            "the fixture contains exactly one face"
        );
        let face = db.faces().next().unwrap().id;
        db.set_sans_serif_family(FAMILY);
        Rc::new(RefCell::new(Self {
            fonts: FontSystem::new_with_locale_and_db("zh-CN".into(), db),
            face,
            counts: Counts::default(),
        }))
    }

    fn shape(&mut self, text: &str, style: Style, width: Option<u32>) -> Result<Paragraph> {
        let mut buffer = Buffer::new_empty(cosmic_text::Metrics::new(
            f32::from(style.size),
            f32::from(style.line_height),
        ));
        buffer.set_wrap(Wrap::WordOrGlyph);
        buffer.set_size(width.map(|w| w as f32), None);
        buffer.set_text(
            text,
            &Attrs::new().family(Family::Name(FAMILY)),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.fonts, false);
        self.counts.shapes += 1;
        let mut lines = Vec::new();
        let mut content = Size::default();
        for run in buffer.layout_runs() {
            let mut glyphs = Vec::with_capacity(run.glyphs.len());
            for glyph in run.glyphs {
                // Missing glyphs are errors, never silent evidence of CJK support.
                if glyph.glyph_id == 0 || glyph.font_id != self.face {
                    return Err(LayoutError::Node("glyph unavailable"));
                }
                glyphs.push(Glyph {
                    start: glyph.start,
                    end: glyph.end,
                    id: glyph.glyph_id,
                    x: glyph.x,
                    y: glyph.y,
                    width: glyph.w,
                    x_offset: glyph.x_offset,
                    y_offset: glyph.y_offset,
                });
            }
            content.width = content.width.max(extent(run.line_w)?);
            content.height = content
                .height
                .max(extent((run.line_top + run.line_height).max(run.line_y))?);
            lines.push(Line {
                source_line: run.line_i,
                top: run.line_top,
                baseline: run.line_y,
                height: run.line_height,
                width: run.line_w,
                glyphs,
            });
        }
        let baseline = lines.first().map(|l| extent(l.baseline)).transpose()?;
        Ok(Paragraph {
            text: text.into(),
            style,
            lines,
            content,
            baseline,
        })
    }
}

struct Cached {
    width: Option<u32>,
    paragraph: Rc<Paragraph>,
}

pub struct Text {
    engine: SharedEngine,
    content: String,
    style: Style,
    cache: RefCell<VecDeque<Cached>>,
    cache_enabled: bool,
}

impl Text {
    pub fn new(
        engine: SharedEngine,
        content: &str,
        style: Style,
        cache_enabled: bool,
    ) -> Result<Self> {
        style.validate()?;
        if content.len() > MAX_TEXT_BYTES {
            return Err(LayoutError::InvalidSize);
        }
        Ok(Self {
            engine,
            content: content.into(),
            style,
            cache: RefCell::default(),
            cache_enabled,
        })
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn set_content(&mut self, content: &str) -> Result<()> {
        if content.len() > MAX_TEXT_BYTES {
            return Err(LayoutError::InvalidSize);
        }
        if self.content() != content {
            self.content = content.into();
            self.release_cache();
        }
        Ok(())
    }

    pub fn set_style(&mut self, style: Style) -> Result<()> {
        style.validate()?;
        if self.style != style {
            self.style = style;
            self.release_cache();
        }
        Ok(())
    }

    pub fn style(&self) -> Style {
        self.style
    }

    pub fn release_cache(&self) {
        *self.cache.borrow_mut() = VecDeque::new();
    }

    pub fn cache_usage(&self) -> (usize, usize) {
        let cache = self.cache.borrow();
        (
            cache.len(),
            cache.capacity() * std::mem::size_of::<Cached>()
                + cache
                    .iter()
                    .map(|c| c.paragraph.retained_bytes())
                    .sum::<usize>(),
        )
    }

    pub fn resolve(&self, query: Query) -> Result<Rc<Paragraph>> {
        let width = match query {
            Query::Natural => None,
            Query::AtMost(w) | Query::Exact(w) => Some(w),
        };
        if width.is_some_and(|w| w > MAX_WIDTH) {
            return Err(LayoutError::InvalidSize);
        }
        let mut engine = self.engine.borrow_mut();
        engine.counts.requests += 1;
        let mut cache = self.cache.borrow_mut();
        if let Some(cached) = cache.iter().find(|c| c.width == width) {
            engine.counts.hits += 1;
            return Ok(Rc::clone(&cached.paragraph));
        }
        let paragraph = Rc::new(engine.shape(&self.content, self.style, width)?);
        if self.cache_enabled {
            if cache.len() == CACHE_SLOTS {
                cache.pop_front();
            }
            cache.push_back(Cached {
                width,
                paragraph: Rc::clone(&paragraph),
            });
        }
        Ok(paragraph)
    }
}

impl LayoutNode for Text {
    fn measure(&mut self, _cx: &mut MeasureContext<'_, '_>, query: Query) -> Result<Metrics> {
        Ok(self.resolve(query)?.metrics(query))
    }

    fn layout(&mut self, _cx: &mut LayoutContext<'_, '_>, assigned: Size) -> Result<LayoutOutput> {
        let paragraph = self.resolve(Query::Exact(assigned.width))?;
        Ok(LayoutOutput {
            baseline: paragraph.baseline,
            content: paragraph.content,
        })
    }
}
