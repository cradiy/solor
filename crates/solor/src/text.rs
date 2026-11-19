use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use crate::{Pixels, SharedString, px};
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Shaping, Wrap, fontdb};

use crate::layout::{
    LayoutContext, LayoutError, LayoutNode, LayoutOutput, MeasureContext, Metrics, Query, Result,
    Size,
};

/// Maximum number of shaped widths retained by one text node.
pub const CACHE_SLOTS: usize = 3;
/// Maximum UTF-8 source size accepted by one text node.
pub const MAX_TEXT_BYTES: usize = 4096;
const MAX_WIDTH: Pixels = px((1 << 20) as f32);

/// Font metrics used to shape a paragraph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    /// Em size in logical pixels.
    pub size: Pixels,
    /// Distance between successive lines in logical pixels.
    pub line_height: Pixels,
}

impl TextStyle {
    fn validate(self) -> Result<()> {
        if !self.size.is_extent() || !self.line_height.is_extent() {
            return Err(LayoutError::InvalidSize);
        }
        if self.size.get() == 0.0
            || self.size.get() > 128.0
            || self.line_height < self.size
            || self.line_height.get() > 256.0
        {
            return Err(LayoutError::InvalidSize);
        }
        Ok(())
    }
}

/// Shaped glyph and the source cluster it represents.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    /// Start byte of the cluster within its source line.
    pub start: usize,
    /// Exclusive end byte of the cluster within its source line.
    pub end: usize,
    /// Glyph index in the selected face.
    pub id: u16,
    /// Horizontal position within the line.
    pub x: f32,
    /// Vertical offset from the line baseline.
    pub y: f32,
    /// Horizontal advance in logical pixels.
    pub width: f32,
    /// Shaping adjustment along the x axis in em units.
    pub x_offset: f32,
    /// Shaping adjustment along the y axis in em units.
    pub y_offset: f32,
}

/// One visual line in a shaped paragraph.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// Index of the source line that produced this visual line.
    pub source_line: usize,
    /// Top edge in paragraph-local coordinates.
    pub top: f32,
    /// Baseline in paragraph-local coordinates.
    pub baseline: f32,
    /// Height of the line box.
    pub height: f32,
    /// Shaped width of the line.
    pub width: f32,
    /// Glyphs in drawing order.
    pub glyphs: Vec<Glyph>,
}

/// Owned immutable layout data; no borrowed Buffer or FontSystem state.
#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    /// Shared source text used for shaping.
    pub text: SharedString,
    /// Font metrics used to shape these lines.
    pub style: TextStyle,
    /// Visual lines after wrapping.
    pub lines: Vec<Line>,
    /// Logical-pixel extent of the shaped content.
    pub content: Size,
    /// First line's baseline in logical pixels.
    pub baseline: Option<Pixels>,
}

impl Paragraph {
    /// Applies a width constraint to this paragraph's metrics.
    pub fn metrics(&self, query: Query) -> Metrics {
        Metrics {
            size: Size {
                width: query.resolve(self.content.width),
                height: self.content.height,
            },
            baseline: self.baseline,
        }
    }

    // The source backing is shared with the node and adds no cache-owned allocation.
    /// Estimates storage retained by this paragraph and its line and glyph vectors.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.lines.capacity() * std::mem::size_of::<Line>()
            + self
                .lines
                .iter()
                .map(|l| l.glyphs.capacity() * std::mem::size_of::<Glyph>())
                .sum::<usize>()
    }
}

/// Counters for shaped-paragraph requests.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    /// Number of paragraph resolution requests.
    pub requests: usize,
    /// Number of requests satisfied by cached paragraphs.
    pub hits: usize,
    /// Number of shaping operations.
    pub shapes: usize,
}

/// Single-face shaping engine shared between text nodes.
pub struct Engine {
    pub(crate) bytes: Arc<Vec<u8>>,
    fonts: FontSystem,
    face: fontdb::ID,
    /// Cumulative request, cache and shaping counts.
    pub counts: Counts,
}

/// Shared ownership of a shaping engine on its UI thread.
pub type SharedEngine = Rc<RefCell<Engine>>;

fn extent(value: f32) -> Result<Pixels> {
    if !value.is_finite() || !(0.0..=(1 << 24) as f32).contains(&value) {
        return Err(LayoutError::InvalidSize);
    }
    Ok(px(value))
}

impl Engine {
    /// Loads one standalone font face. All text in this engine uses that face.
    pub fn from_font(bytes: Vec<u8>, locale: &str) -> Result<SharedEngine> {
        let mut db = fontdb::Database::new();
        let bytes = Arc::new(bytes);
        db.load_font_source(fontdb::Source::Binary(bytes.clone()));
        let mut faces = db.faces();
        let face = faces
            .next()
            .ok_or(LayoutError::Node("font contains no face"))?;
        let family = face
            .families
            .first()
            .ok_or(LayoutError::Node("font has no family"))?
            .0
            .clone();
        let face = face.id;
        if faces.next().is_some() {
            return Err(LayoutError::Node("expected a single font face"));
        }
        drop(faces);
        db.set_sans_serif_family(family);
        Ok(Rc::new(RefCell::new(Self {
            bytes,
            fonts: FontSystem::new_with_locale_and_db(locale.into(), db),
            face,
            counts: Counts::default(),
        })))
    }

    fn shape(
        &mut self,
        text: &SharedString,
        style: TextStyle,
        width: Option<Pixels>,
    ) -> Result<Paragraph> {
        let mut buffer = Buffer::new_empty(cosmic_text::Metrics::new(
            style.size.get(),
            style.line_height.get(),
        ));
        buffer.set_wrap(Wrap::WordOrGlyph);
        buffer.set_size(width.map(Pixels::get), None);
        buffer.set_text(
            text,
            &Attrs::new().family(Family::SansSerif),
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
            text: text.clone(),
            style,
            lines,
            content,
            baseline,
        })
    }
}

struct Cached {
    width: Option<Pixels>,
    paragraph: Rc<Paragraph>,
}

/// Retained text leaf with a bounded paragraph cache.
pub struct Text {
    engine: SharedEngine,
    content: SharedString,
    style: TextStyle,
    cache: RefCell<VecDeque<Cached>>,
    cache_enabled: bool,
}

impl Text {
    /// Validates the source and metrics and creates a text leaf.
    pub fn new(
        engine: SharedEngine,
        content: impl AsRef<str> + Into<SharedString>,
        style: TextStyle,
        cache_enabled: bool,
    ) -> Result<Self> {
        style.validate()?;
        Self::validate_content(content.as_ref())?;
        Ok(Self {
            engine,
            content: content.into(),
            style,
            cache: RefCell::default(),
            cache_enabled,
        })
    }

    /// Borrows the current UTF-8 source.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Replaces changed source text and releases its cached paragraphs.
    pub fn set_content(&mut self, content: impl AsRef<str> + Into<SharedString>) -> Result<()> {
        Self::validate_content(content.as_ref())?;
        if self.content() != content.as_ref() {
            self.replace_content(content.into());
        }
        Ok(())
    }

    pub(crate) fn validate_content(content: &str) -> Result<()> {
        if content.len() > MAX_TEXT_BYTES {
            return Err(LayoutError::InvalidSize);
        }
        Ok(())
    }

    pub(crate) fn replace_content(&mut self, content: SharedString) {
        self.content = content;
        self.release_cache();
    }

    /// Applies changed font metrics and releases stale paragraphs.
    pub fn set_style(&mut self, style: TextStyle) -> Result<()> {
        style.validate()?;
        if self.style != style {
            self.style = style;
            self.release_cache();
        }
        Ok(())
    }

    /// Returns the current shaping metrics.
    pub fn style(&self) -> TextStyle {
        self.style
    }

    /// Releases the node's paragraph cache.
    pub fn release_cache(&self) {
        *self.cache.borrow_mut() = VecDeque::new();
    }

    /// Returns the cached paragraph count and estimated retained bytes.
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

    /// Resolves a paragraph for the requested width, shaping on a cache miss.
    pub fn resolve(&self, query: Query) -> Result<Rc<Paragraph>> {
        let width = match query {
            Query::Natural => None,
            Query::AtMost(w) | Query::Exact(w) => Some(w),
        };
        if width.is_some_and(|w| !w.is_extent() || w > MAX_WIDTH) {
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
