use std::{fmt, ops::Deref};

use smol_str::SmolStr;

/// Immutable UI text. Cloning a long string shares its backing storage.
#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SharedString(SmolStr);

impl SharedString {
    /// Constructs shared text from a static string literal.
    pub const fn from_static(text: &'static str) -> Self {
        Self(SmolStr::new_static(text))
    }

    /// Borrows the UTF-8 contents.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for SharedString {
    fn from(text: &str) -> Self {
        Self(text.into())
    }
}

impl From<String> for SharedString {
    fn from(text: String) -> Self {
        Self(text.into())
    }
}

impl From<&String> for SharedString {
    fn from(text: &String) -> Self {
        Self::from(text.as_str())
    }
}

impl From<&SharedString> for SharedString {
    fn from(text: &SharedString) -> Self {
        text.clone()
    }
}

impl AsRef<str> for SharedString {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for SharedString {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<&str> for SharedString {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl fmt::Display for SharedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl fmt::Debug for SharedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}
