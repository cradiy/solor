//! Font discovery, registration and retained face data.
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use cosmic_text::{Family, FontSystem, fontdb};

use crate::{
    SharedString,
    layout::{LayoutError, Result},
};

use super::{FontFamily, GenericFontFamily, family::FamilyPreferences};

/// Font sources and preferences used when creating a UI or text engine.
pub struct Fonts {
    pub(super) db: fontdb::Database,
    locale: String,
    preferences: FamilyPreferences,
    system_fonts: bool,
}

impl Fonts {
    /// Discovers installed fonts in the directories supported by the host font database.
    pub fn system() -> Self {
        let mut fonts = Self::empty();
        fonts.db.load_system_fonts();
        fonts.system_fonts = true;
        fonts
    }

    /// Creates an isolated collection for application-supplied fonts.
    pub fn empty() -> Self {
        Self {
            db: fontdb::Database::new(),
            preferences: FamilyPreferences::default(),
            system_fonts: false,
            locale: sys_locale::get_locale().unwrap_or_else(|| "en-US".into()),
        }
    }

    /// Registers a font file or collection from memory and returns its first family name.
    pub fn add_font(&mut self, bytes: Vec<u8>) -> Result<SharedString> {
        let ids = self
            .db
            .load_font_source(fontdb::Source::Binary(Arc::new(bytes)));
        let face = ids
            .first()
            .and_then(|id| self.db.face(*id))
            .ok_or(LayoutError::Node("font contains no usable faces"))?;
        Ok(face.families[0].0.as_str().into())
    }

    /// Reads and registers a font file or collection.
    pub fn load_font(&mut self, path: impl AsRef<Path>) -> anyhow::Result<SharedString> {
        Ok(self.add_font(std::fs::read(path)?)?)
    }

    /// Returns the sorted family names available for explicit selection.
    pub fn families(&self) -> Vec<&str> {
        let mut names: Vec<_> = self
            .db
            .faces()
            .flat_map(|f| f.families.iter().map(|(name, _)| name.as_str()))
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Selects the family inherited by unstyled text.
    pub fn set_default_family(&mut self, family: impl Into<FontFamily>) -> Result<()> {
        let family = family.into();
        if let FontFamily::Named(name) = &family {
            self.check_family(name)?;
        }
        self.preferences.default = family;
        Ok(())
    }

    /// Maps a generic category to a literal name for primary and fallback selection.
    /// Passing `None` restores platform resolution for that category.
    pub fn set_generic_font_family(
        &mut self,
        family: GenericFontFamily,
        name: Option<SharedString>,
    ) {
        if let Some(name) = name {
            self.preferences.generics.insert(family, name);
        } else {
            self.preferences.generics.remove(&family);
        }
    }

    fn check_family(&self, family: &str) -> Result<()> {
        if self
            .db
            .faces()
            .any(|f| f.families.iter().any(|(name, _)| name == family))
        {
            Ok(())
        } else {
            Err(LayoutError::Node("unknown font family"))
        }
    }

    pub(super) fn apply_platform_family(&mut self, provider: Option<fn() -> Option<String>>) {
        if self.system_fonts
            && !self
                .preferences
                .generics
                .contains_key(&GenericFontFamily::SystemUi)
            && let Some(name) = provider.and_then(|read| read())
            && self.check_family(&name).is_ok()
        {
            self.preferences
                .generics
                .insert(GenericFontFamily::SystemUi, name.into());
        }
    }

    /// Overrides the system language used to choose script-specific fallback families.
    pub fn set_locale(&mut self, locale: impl Into<String>) {
        self.locale = locale.into();
    }

    fn missing_alias(
        &self,
        category: Family<'_>,
        candidates: &[&'static str],
    ) -> Option<&'static str> {
        // A platform alias can name an uninstalled family.
        if self.check_family(self.db.family_name(&category)).is_ok() {
            return None;
        }
        candidates
            .iter()
            .copied()
            .find(|name| self.check_family(name).is_ok())
    }

    fn resolve_generic_families(&mut self) {
        if let Some(name) = self.missing_alias(
            Family::SansSerif,
            &[
                "Segoe UI",
                "Helvetica Neue",
                "Noto Sans",
                "Cantarell",
                "DejaVu Sans",
                "Liberation Sans",
                "Arial",
            ],
        ) {
            self.db.set_sans_serif_family(name);
        } else if self.db.query(&sans_query()).is_none() {
            let first = self
                .db
                .faces()
                .next()
                .and_then(|f| f.families.first())
                .map(|(name, _)| name.clone());
            if let Some(first) = first {
                self.db.set_sans_serif_family(first);
            }
        }
        if let Some(name) = self.missing_alias(
            Family::Serif,
            &[
                "Noto Serif",
                "DejaVu Serif",
                "Liberation Serif",
                "Times New Roman",
            ],
        ) {
            self.db.set_serif_family(name);
        }
        if let Some(name) =
            self.missing_alias(Family::Cursive, &["Comic Sans MS", "URW Chancery L"])
        {
            self.db.set_cursive_family(name);
        }
        if let Some(name) = self.missing_alias(Family::Fantasy, &["Impact", "Papyrus"]) {
            self.db.set_fantasy_family(name);
        }
        let configured = self.db.family_name(&Family::Monospace);
        if self
            .db
            .faces()
            .any(|f| f.monospaced && f.families.iter().any(|(name, _)| name == configured))
        {
            return;
        }
        let fixed = self
            .db
            .faces()
            .filter(|f| f.monospaced)
            .flat_map(|f| f.families.iter().map(|(name, _)| name))
            .min()
            .cloned();
        if let Some(name) = fixed {
            self.db.set_monospace_family(name);
        }
    }

    pub(super) fn into_system(mut self) -> (FontSystem, FamilyPreferences) {
        self.resolve_generic_families();
        (
            FontSystem::new_with_locale_and_db(self.locale, self.db),
            self.preferences,
        )
    }
}

pub(super) fn sans_query() -> fontdb::Query<'static> {
    fontdb::Query {
        families: &[Family::SansSerif],
        ..Default::default()
    }
}

/// Immutable face used to shape and rasterize a paragraph's glyphs.
#[derive(Clone, Debug)]
pub struct FontFace {
    pub(super) font: Arc<cosmic_text::Font>,
    index: u32,
    weight: u16,
    key: u64,
}

impl FontFace {
    pub(super) fn new(font: Arc<cosmic_text::Font>, index: u32, weight: u16) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            font,
            index,
            weight,
            key: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }
    /// Complete file backing this face, retained for the lifetime of the paragraph.
    pub fn data(&self) -> &[u8] {
        self.font.data()
    }
    /// Face index within its font collection.
    pub fn index(&self) -> u32 {
        self.index
    }
    /// Weight used during shaping, including the weight axis of a variable font.
    pub fn weight(&self) -> u16 {
        self.weight
    }
    /// Process-local key for raster caches, including the face's weight.
    pub fn cache_key(&self) -> u64 {
        self.key
    }
}

impl PartialEq for FontFace {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            || (self.index == other.index
                && self.weight == other.weight
                && self.data() == other.data())
    }
}
