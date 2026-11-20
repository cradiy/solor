use std::cell::RefCell;
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::rc::Rc;
use std::sync::Arc;

use crate::style::{TextAlign, TextAttributes, TextOverflow, WhiteSpace};
use crate::{Pixels, SharedString, px};
use cosmic_text::{
    Align, Attrs, Buffer, BufferLine, Ellipsize, EllipsizeHeightLimit, Family, FontSystem,
    LayoutGlyph, LayoutRun, LineEnding, Shaping, Wrap, fontdb,
};

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
    /// Start byte from the beginning of the visual line's source paragraph.
    pub start: usize,
    /// Exclusive end byte in the source paragraph. An ellipsis has an empty range at the elision boundary.
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
    /// Index of the source paragraph. Folding line breaks makes the entire input one paragraph.
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
    /// Visible visual lines after wrapping and applying the line limit.
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

#[derive(Clone, Copy, PartialEq)]
struct ParagraphStyle {
    alignment: TextAlign,
    line_clamp: Option<NonZeroUsize>,
    white_space: WhiteSpace,
    overflow: TextOverflow,
}

impl ParagraphStyle {
    fn from_attributes(attrs: Option<&TextAttributes>) -> Self {
        Self {
            alignment: attrs.and_then(|a| a.text_align).unwrap_or(TextAlign::Start),
            line_clamp: attrs.and_then(|a| a.line_clamp.flatten()),
            white_space: attrs
                .and_then(|a| a.white_space)
                .unwrap_or(WhiteSpace::PreWrap),
            overflow: attrs
                .and_then(|a| a.text_overflow)
                .unwrap_or(TextOverflow::Clip),
        }
    }
}

// Each display byte boundary maps back into the unmodified UTF-8 source.
struct CollapsedText {
    display: String,
    offsets: Vec<usize>,
}

impl CollapsedText {
    fn new(source: &str) -> Option<Self> {
        let bytes = source.as_bytes();
        let needs_folding = bytes.iter().enumerate().any(|(i, &b)| {
            b.is_ascii_whitespace()
                && (b != b' ' || i == 0 || i + 1 == bytes.len() || bytes[i - 1] == b' ')
        });
        if !needs_folding {
            return None;
        }
        let mut display = String::with_capacity(bytes.len());
        let mut offsets = Vec::with_capacity(bytes.len() + 1);
        let mut space_start = None;
        for (i, ch) in source.char_indices() {
            if ch.is_ascii_whitespace() {
                space_start.get_or_insert(i);
                continue;
            }
            if let Some(start) = space_start.take()
                && !display.is_empty()
            {
                display.push(' ');
                offsets.push(start);
            }
            display.push(ch);
            offsets.extend(i..i + ch.len_utf8());
        }
        offsets.push(space_start.unwrap_or(bytes.len()));
        Some(Self { display, offsets })
    }
}

/// Single-face shaping engine shared between text nodes.
pub struct Engine {
    pub(crate) bytes: Arc<Vec<u8>>,
    fonts: FontSystem,
    face: fontdb::ID,
    ellipsis: &'static str,
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
        let has_ellipsis = ttf_parser::Face::parse(bytes.as_slice(), face.index)
            .map_err(|_| LayoutError::Node("invalid font face"))?
            .glyph_index('…')
            .is_some();
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
            ellipsis: if has_ellipsis { "…" } else { "..." },
            counts: Counts::default(),
        })))
    }

    fn glyphs<'a>(
        &self,
        glyphs: impl Iterator<Item = &'a LayoutGlyph>,
        offset: impl Fn(usize) -> usize,
    ) -> Result<Vec<Glyph>> {
        let mut result = Vec::with_capacity(glyphs.size_hint().1.unwrap_or(0));
        for glyph in glyphs {
            if glyph.glyph_id == 0 || glyph.font_id != self.face {
                return Err(LayoutError::Node("glyph unavailable"));
            }
            result.push(Glyph {
                start: offset(glyph.start),
                end: offset(glyph.end),
                id: glyph.glyph_id,
                x: glyph.x,
                y: glyph.y,
                width: glyph.w,
                x_offset: glyph.x_offset,
                y_offset: glyph.y_offset,
            });
        }
        Ok(result)
    }

    fn marked_run(
        &mut self,
        buffer: &Buffer,
        run: &LayoutRun<'_>,
        source_offset: impl Fn(usize) -> usize,
    ) -> Result<(Vec<Glyph>, f32)> {
        // Glyphs may be in visual order. Shape the corresponding logical slice
        // with the original paragraph direction and the marker in the same line.
        let start = run.glyphs.iter().map(|g| g.start).min().unwrap_or(0);
        let end = run.glyphs.iter().map(|g| g.end).max().unwrap_or(start);
        let direction = if run.rtl { '\u{200f}' } else { '\u{200e}' };
        let mut marked =
            String::with_capacity(direction.len_utf8() + end - start + self.ellipsis.len());
        marked.push(direction);
        marked.push_str(&run.text[start..end]);
        marked.push_str(self.ellipsis);
        let original = &buffer.lines[run.line_i];
        let mut line = BufferLine::new(
            marked,
            LineEnding::None,
            original.attrs_list().clone(),
            Shaping::Advanced,
        );
        line.set_align(original.align());
        let [layout] = line.layout(
            &mut self.fonts,
            buffer.metrics().font_size,
            buffer.size().0,
            Wrap::None,
            Ellipsize::End(EllipsizeHeightLimit::Lines(1)),
            None,
            buffer.tab_width(),
            buffer.hinting(),
        ) else {
            return Err(LayoutError::Node("invalid ellipsis layout"));
        };
        let prefix = direction.len_utf8();
        let visible = layout
            .glyphs
            .iter()
            .filter(|g| g.start >= prefix || g.start == g.end);
        let glyphs = self.glyphs(visible, |i| {
            source_offset((start + i.saturating_sub(prefix)).min(end))
        })?;
        Ok((glyphs, layout.w))
    }

    fn shape(
        &mut self,
        text: &SharedString,
        style: TextStyle,
        paragraph: ParagraphStyle,
        width: Option<Pixels>,
    ) -> Result<Paragraph> {
        let mut buffer = Buffer::new_empty(cosmic_text::Metrics::new(
            style.size.get(),
            style.line_height.get(),
        ));
        let nowrap = matches!(paragraph.white_space, WhiteSpace::NoWrap | WhiteSpace::Pre);
        let collapsed = match paragraph.white_space {
            WhiteSpace::Normal | WhiteSpace::NoWrap => CollapsedText::new(text),
            _ => None,
        };
        let display = collapsed
            .as_ref()
            .map_or(text.as_str(), |c| c.display.as_str());
        buffer.set_wrap(if nowrap {
            Wrap::None
        } else {
            Wrap::WordOrGlyph
        });
        if nowrap && width.is_some() {
            let limit = EllipsizeHeightLimit::Lines(1);
            buffer.set_ellipsize(match paragraph.overflow {
                TextOverflow::Ellipsis => Ellipsize::End(limit),
                TextOverflow::EllipsisStart => Ellipsize::Start(limit),
                TextOverflow::EllipsisMiddle => Ellipsize::Middle(limit),
                TextOverflow::Clip => Ellipsize::None,
            });
        }
        let line_clamp = paragraph.line_clamp;
        let mark_overflow =
            !nowrap && paragraph.overflow == TextOverflow::Ellipsis && line_clamp.is_some();
        // One line of lookahead distinguishes an exact fit from hidden content.
        let height = line_clamp.map(|count| {
            (count.get().saturating_add(usize::from(mark_overflow)) as f32)
                * style.line_height.get()
        });
        buffer.set_size(width.map(Pixels::get), height);
        buffer.set_text(
            display,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            match paragraph.alignment {
                TextAlign::Start => None,
                TextAlign::End => Some(Align::End),
                TextAlign::Left => Some(Align::Left),
                TextAlign::Right => Some(Align::Right),
                TextAlign::Center => Some(Align::Center),
                TextAlign::Justify => Some(Align::Justified),
            },
        );
        buffer.shape_until_scroll(&mut self.fonts, false);
        self.counts.shapes += 1;
        let mut lines = Vec::new();
        let mut content = Size::default();
        let limit = line_clamp.map_or(usize::MAX, NonZeroUsize::get);
        let mut runs = buffer.layout_runs().peekable();
        while lines.len() < limit {
            let Some(run) = runs.next() else { break };
            let source_offset = |i: usize| collapsed.as_ref().map_or(i, |c| c.offsets[i]);
            let (glyphs, line_width) =
                if mark_overflow && lines.len() + 1 == limit && runs.peek().is_some() {
                    self.marked_run(&buffer, &run, source_offset)?
                } else {
                    (self.glyphs(run.glyphs.iter(), source_offset)?, run.line_w)
                };
            content.width = content.width.max(extent(line_width)?);
            content.height = content
                .height
                .max(extent((run.line_top + run.line_height).max(run.line_y))?);
            lines.push(Line {
                source_line: run.line_i,
                top: run.line_top,
                baseline: run.line_y,
                height: run.line_height,
                width: line_width,
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
    paragraph: ParagraphStyle,
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
            paragraph: ParagraphStyle::from_attributes(None),
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

    pub(crate) fn set_paragraph_style(&mut self, attributes: Option<&TextAttributes>) {
        let paragraph = ParagraphStyle::from_attributes(attributes);
        if self.paragraph != paragraph {
            self.paragraph = paragraph;
            self.release_cache();
        }
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
        let paragraph = Rc::new(engine.shape(&self.content, self.style, self.paragraph, width)?);
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
