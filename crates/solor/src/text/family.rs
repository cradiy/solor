//! Font family selectors used by element styles and font preferences.
use std::collections::HashMap;

use cosmic_text::Family;

use crate::SharedString;

/// A font category resolved by the platform or application.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum GenericFontFamily {
    /// The platform's interface font.
    #[default]
    SystemUi,
    /// A serif face.
    Serif,
    /// A sans-serif face.
    SansSerif,
    /// A fixed-pitch face.
    Monospace,
    /// A cursive face.
    Cursive,
    /// A decorative face.
    Fantasy,
}

/// A literal family name or a platform-resolved category.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum FontFamily {
    /// A name from a font file's family table.
    Named(SharedString),
    /// A generic font category.
    Generic(GenericFontFamily),
}

impl Default for FontFamily {
    fn default() -> Self {
        GenericFontFamily::SystemUi.into()
    }
}

impl From<GenericFontFamily> for FontFamily {
    fn from(category: GenericFontFamily) -> Self {
        Self::Generic(category)
    }
}

impl<T: Into<SharedString>> From<T> for FontFamily {
    fn from(name: T) -> Self {
        Self::Named(name.into())
    }
}

#[derive(Default)]
pub(super) struct FamilyPreferences {
    pub default: FontFamily,
    pub generics: HashMap<GenericFontFamily, SharedString>,
}

impl FamilyPreferences {
    pub fn resolve<'a>(&'a self, family: Option<&'a FontFamily>) -> Family<'a> {
        let generic = match family.unwrap_or(&self.default) {
            FontFamily::Named(name) => return Family::Name(name),
            FontFamily::Generic(generic) => generic,
        };
        if let Some(name) = self.generics.get(generic) {
            return Family::Name(name);
        }
        match generic {
            GenericFontFamily::SystemUi | GenericFontFamily::SansSerif => Family::SansSerif,
            GenericFontFamily::Serif => Family::Serif,
            GenericFontFamily::Monospace => Family::Monospace,
            GenericFontFamily::Cursive => Family::Cursive,
            GenericFontFamily::Fantasy => Family::Fantasy,
        }
    }
}
