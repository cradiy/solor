mod decoration;
mod family;
mod fonts;
use decoration::Decorations;
pub(crate) use decoration::Stroke;
use family::FamilyPreferences;
pub use family::{FontFamily, GenericFontFamily};
use fonts::sans_query;
pub use fonts::{FontFace, Fonts};

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::num::NonZeroUsize;
use std::rc::Rc;

use crate::style::{FontFeature, TextAlign, TextAttributes, TextOverflow, WhiteSpace};
use crate::{Pixels, SharedString, px};
use cosmic_text::{
    Align, Attrs, Buffer, BufferLine, Ellipsize, EllipsizeHeightLimit, FontSystem, LayoutGlyph,
    LayoutRun, LineEnding, Shaping, Wrap, fontdb,
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

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: px(16.0),
            line_height: px(24.0),
        }
    }
}

impl TextStyle {
    pub(crate) fn validate(self) -> Result<()> {
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
    /// Index of the selected face in the paragraph's face table.
    pub face: usize,
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
    /// Faces referenced by the glyphs, retained even after the UI is dropped.
    pub faces: Vec<FontFace>,
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
            + self.faces.capacity() * std::mem::size_of::<FontFace>()
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

#[derive(PartialEq)]
struct ParagraphStyle {
    family: Option<FontFamily>,
    fallbacks: Option<Rc<[FontFamily]>>,
    weight: fontdb::Weight,
    slant: fontdb::Style,
    features: Option<Rc<[FontFeature]>>,
    letter_spacing: Pixels,
    alignment: TextAlign,
    line_clamp: Option<NonZeroUsize>,
    white_space: WhiteSpace,
    overflow: TextOverflow,
}

impl ParagraphStyle {
    fn from_attributes(attrs: Option<&TextAttributes>, letter_spacing: Pixels) -> Self {
        Self {
            family: attrs.and_then(|a| a.font_family.clone()),
            fallbacks: attrs.and_then(|a| a.font_fallbacks.clone()),
            weight: fontdb::Weight(attrs.and_then(|a| a.font_weight).unwrap_or(400)),
            slant: match attrs.and_then(|a| a.font_style) {
                Some(crate::style::FontStyle::Italic) => fontdb::Style::Italic,
                Some(crate::style::FontStyle::Oblique) => fontdb::Style::Oblique,
                _ => fontdb::Style::Normal,
            },
            features: attrs.and_then(|a| a.font_features.clone()),
            letter_spacing,
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

/// Font matching and shaping shared between text nodes.
pub struct Engine {
    fonts: FontSystem,
    preferences: FamilyPreferences,
    faces: HashMap<(fontdb::ID, u16), FontFace>,
    ellipsis: &'static str,
    /// Cumulative request, cache and shaping counts.
    pub counts: Counts,
}

/// Shared ownership of a shaping engine on its UI thread.
pub type SharedEngine = Rc<RefCell<Engine>>;

#[derive(Default)]
pub(crate) struct TextSystem {
    engine: std::cell::OnceCell<SharedEngine>,
    collection: std::cell::Cell<Option<Box<Fonts>>>,
    pub(crate) platform_family: Option<fn() -> Option<String>>,
}

impl TextSystem {
    pub(crate) fn with_fonts(collection: Fonts) -> Self {
        Self {
            collection: Some(Box::new(collection)).into(),
            ..Self::default()
        }
    }

    pub(crate) fn engine(&self) -> &SharedEngine {
        self.engine.get_or_init(|| {
            let mut fonts = self
                .collection
                .take()
                .map_or_else(Fonts::system, |fonts| *fonts);
            fonts.apply_platform_family(self.platform_family);
            Engine::with_fonts(fonts)
        })
    }
}

fn extent(value: f32) -> Result<Pixels> {
    if !value.is_finite() || !(0.0..=(1 << 24) as f32).contains(&value) {
        return Err(LayoutError::InvalidSize);
    }
    Ok(px(value))
}

impl Engine {
    /// Creates a shaping engine using the registered faces and locale preferences.
    pub fn with_fonts(collection: Fonts) -> SharedEngine {
        let (fonts, preferences) = collection.into_system();
        let db = fonts.db();
        let primary = db.query(&fontdb::Query {
            families: &[preferences.resolve(None)],
            ..Default::default()
        });
        let ellipsis = primary
            .and_then(|id| {
                db.with_face_data(id, |bytes, index| {
                    ttf_parser::Face::parse(bytes, index)
                        .ok()
                        .and_then(|face| face.glyph_index('…'))
                        .is_some()
                })
            })
            .unwrap_or(false);
        Rc::new(RefCell::new(Self {
            fonts,
            preferences,
            faces: HashMap::new(),
            ellipsis: if ellipsis { "…" } else { "..." },
            counts: Counts::default(),
        }))
    }

    /// Creates a shaping engine with the supplied font as its default family.
    pub fn from_font(bytes: Vec<u8>) -> Result<SharedEngine> {
        let mut fonts = Fonts::empty();
        let family = fonts.add_font(bytes)?;
        fonts.set_default_family(&family)?;
        Ok(Self::with_fonts(fonts))
    }

    fn glyphs<'a>(
        &mut self,
        faces: &mut Vec<FontFace>,
        glyphs: impl Iterator<Item = &'a LayoutGlyph>,
        offset: impl Fn(usize) -> usize,
    ) -> Result<Vec<Glyph>> {
        let mut result = Vec::with_capacity(glyphs.size_hint().1.unwrap_or(0));
        for glyph in glyphs {
            if glyph.glyph_id == 0 {
                return Err(LayoutError::Node("glyph unavailable"));
            }
            if !glyph.x.is_finite() || !glyph.w.is_finite() {
                return Err(LayoutError::InvalidSize);
            }
            let weight = glyph.font_weight.0;
            let face = if let Some(index) = faces
                .iter()
                .position(|f| f.font.id() == glyph.font_id && f.weight() == weight)
            {
                index
            } else {
                let entry = self.faces.entry((glyph.font_id, weight));
                let resolved = match entry {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let font = self
                            .fonts
                            .get_font(glyph.font_id, glyph.font_weight)
                            .ok_or(LayoutError::Node("font face unavailable"))?;
                        let index = self
                            .fonts
                            .db()
                            .face(glyph.font_id)
                            .expect("shaped face")
                            .index;
                        entry.insert(FontFace::new(font, index, weight))
                    }
                };
                faces.push(resolved.clone());
                faces.len() - 1
            };
            result.push(Glyph {
                face,
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

    fn apply_fallbacks(
        system: &mut FontSystem,
        preferences: &FamilyPreferences,
        buffer: &mut Buffer,
        primary: &Attrs<'_>,
        fallbacks: &[FontFamily],
    ) {
        use unicode_segmentation::UnicodeSegmentation;
        if fallbacks.is_empty() {
            return;
        }
        let fonts: Vec<_> = std::iter::once(primary.family)
            .chain(fallbacks.iter().map(|f| preferences.resolve(Some(f))))
            .filter_map(|family| {
                let db = system.db();
                let id = db.query(&fontdb::Query {
                    families: &[family],
                    weight: primary.weight,
                    style: primary.style,
                    stretch: primary.stretch,
                })?;
                let index = db.face(id)?.index;
                Some((family, system.get_font(id, primary.weight)?, index))
            })
            .collect();
        let coverage: Vec<_> = fonts
            .iter()
            .filter_map(|(family, font, index)| {
                Some((*family, ttf_parser::Face::parse(font.data(), *index).ok()?))
            })
            .collect();
        for line in &mut buffer.lines {
            let mut spans = line.attrs_list().clone();
            for (start, cluster) in line.text().grapheme_indices(true) {
                if let Some((family, _)) = coverage.iter().find(|(_, face)| {
                    cluster
                        .chars()
                        .filter(|c| !matches!(c, '\u{200d}' | '\u{fe0e}' | '\u{fe0f}'))
                        .all(|c| face.glyph_index(c).is_some())
                }) && *family != primary.family
                {
                    spans.add_span(
                        start..start + cluster.len(),
                        &primary.clone().family(*family),
                    );
                }
            }
            line.set_attrs_list(spans);
        }
    }

    fn marked_run(
        &mut self,
        buffer: &Buffer,
        faces: &mut Vec<FontFace>,
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
        let prefix = direction.len_utf8();
        let defaults = original.attrs_list().defaults();
        let mut attrs = cosmic_text::AttrsList::new(&defaults);
        for (range, span) in original.attrs_list().spans_iter() {
            let left = range.start.max(start);
            let right = range.end.min(end);
            if left < right {
                attrs.add_span(
                    prefix + left - start..prefix + right - start,
                    &span.as_attrs(),
                );
            }
        }
        if defaults.letter_spacing_opt.is_some() {
            // The synthetic direction mark must occupy no horizontal space.
            attrs.add_span(0..prefix, &defaults.letter_spacing(0.0));
        }
        let mut line = BufferLine::new(marked, LineEnding::None, attrs, Shaping::Advanced);
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
        let visible = layout
            .glyphs
            .iter()
            .filter(|g| g.start >= prefix || g.start == g.end);
        let glyphs = self.glyphs(faces, visible, |i| {
            source_offset((start + i.saturating_sub(prefix)).min(end))
        })?;
        Ok((glyphs, layout.w))
    }

    fn shape(
        &mut self,
        text: &SharedString,
        style: TextStyle,
        paragraph: &ParagraphStyle,
        width: Option<Pixels>,
    ) -> Result<Paragraph> {
        if self.fonts.db().is_empty() {
            return Err(LayoutError::Node("no fonts available"));
        }
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
        let family = self.preferences.resolve(paragraph.family.as_ref());
        let mut attrs = Attrs::new()
            .family(family)
            .weight(paragraph.weight)
            .style(paragraph.slant);
        if paragraph.letter_spacing != px(0.0) {
            let spacing_em = paragraph.letter_spacing.get() / style.size.get();
            if !spacing_em.is_finite() {
                return Err(LayoutError::InvalidSize);
            }
            attrs = attrs.letter_spacing(spacing_em);
        }
        if let Some(features) = &paragraph.features {
            for feature in features.iter() {
                attrs
                    .font_features
                    .set(cosmic_text::FeatureTag::new(&feature.tag), feature.value);
            }
        }
        buffer.set_text(
            display,
            &attrs,
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
        if let Some(fallbacks) = paragraph.fallbacks.as_deref() {
            Self::apply_fallbacks(
                &mut self.fonts,
                &self.preferences,
                &mut buffer,
                &attrs,
                fallbacks,
            );
        }
        buffer.shape_until_scroll(&mut self.fonts, false);

        self.counts.shapes += 1;
        let mut faces = Vec::new();
        let mut lines = Vec::new();
        let mut content = Size::default();
        let limit = line_clamp.map_or(usize::MAX, NonZeroUsize::get);
        let mut runs = buffer.layout_runs().peekable();
        while lines.len() < limit {
            let Some(run) = runs.next() else { break };
            let source_offset = |i: usize| collapsed.as_ref().map_or(i, |c| c.offsets[i]);
            let (glyphs, line_width) =
                if mark_overflow && lines.len() + 1 == limit && runs.peek().is_some() {
                    self.marked_run(&buffer, &mut faces, &run, source_offset)?
                } else {
                    (
                        self.glyphs(&mut faces, run.glyphs.iter(), source_offset)?,
                        run.line_w,
                    )
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
            faces,
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
    pub(crate) decorations: Option<Box<Decorations>>,
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
            paragraph: ParagraphStyle::from_attributes(None, px(0.0)),
            decorations: None,
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

    pub(crate) fn set_paragraph_style(
        &mut self,
        attributes: Option<&TextAttributes>,
        letter_spacing: Pixels,
    ) {
        let paragraph = ParagraphStyle::from_attributes(attributes, letter_spacing);
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
        let paragraph = Rc::new(engine.shape(&self.content, self.style, &self.paragraph, width)?);
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
