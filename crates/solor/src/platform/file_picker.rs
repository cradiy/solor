use std::path::PathBuf;

use anyhow::{Result, ensure};

/// A named group of filename extensions in a file picker.
#[derive(Clone, Debug)]
pub struct FileFilter {
    /// Label shown on systems with a filter menu.
    pub name: String,
    /// Literal extensions without a leading dot, such as `png` and `jpg`.
    pub extensions: Vec<String>,
}

/// Options for opening an existing file or directory with the system picker.
#[derive(Clone, Debug, Default)]
pub struct FilePickerOptions {
    /// Select directories instead of files.
    pub directories: bool,
    /// Optional dialog title.
    pub title: Option<String>,
    /// Optional label for the confirmation button.
    pub accept_label: Option<String>,
    /// Suggested starting directory.
    pub initial_directory: Option<PathBuf>,
    /// File filters. Linux offers named groups and macOS accepts their combined extensions.
    pub filters: Vec<FileFilter>,
}

impl FilePickerOptions {
    /// Creates an unrestricted file picker.
    pub fn files() -> Self {
        Self::default()
    }

    /// Creates a directory picker.
    pub fn folders() -> Self {
        Self {
            directories: true,
            ..Self::default()
        }
    }

    /// Sets the dialog title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the confirmation button label.
    pub fn accept_label(mut self, label: impl Into<String>) -> Self {
        self.accept_label = Some(label.into());
        self
    }

    /// Suggests the directory shown when the picker opens.
    pub fn initial_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.initial_directory = Some(directory.into());
        self
    }

    /// Adds a named group of file extensions, for example `.filter("Images", ["png", "jpg"])`.
    pub fn filter(
        mut self,
        name: impl Into<String>,
        extensions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.filters.push(FileFilter {
            name: name.into(),
            extensions: extensions.into_iter().map(Into::into).collect(),
        });
        self
    }

    /// Checks that the options can be represented by a native chooser.
    pub fn validate(&self) -> Result<()> {
        for label in self.title.iter().chain(&self.accept_label) {
            ensure!(!label.contains('\0'), "picker labels cannot contain NUL");
        }
        if let Some(dir) = &self.initial_directory {
            ensure!(
                !dir.as_os_str().is_empty() && !dir.as_os_str().as_encoded_bytes().contains(&0),
                "invalid picker directory"
            );
        }
        ensure!(
            !self.directories || self.filters.is_empty(),
            "file filters require a file picker"
        );
        for filter in &self.filters {
            ensure!(
                !filter.name.is_empty() && !filter.name.contains('\0'),
                "invalid file filter name"
            );
            ensure!(
                !filter.extensions.is_empty(),
                "file filter requires at least one extension"
            );
            for ext in &filter.extensions {
                ensure!(
                    !ext.is_empty()
                        && ext
                            .chars()
                            .all(|c| c.is_alphanumeric() || matches!(c, '+' | '-' | '_')),
                    "invalid filename extension {ext:?}"
                );
            }
        }
        Ok(())
    }
}
