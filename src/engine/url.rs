//! A fully-resolved download URL.

use std::fmt;

/// A fully-resolved download URL (placeholders already substituted).
///
/// A plain [`String`] forces every consumer to re-validate that the value is
/// actually a URL; the newtype carries that intent so the download boundary
/// stays honest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url(String);

impl Url {
    /// The URL as a string slice, for rendering and network calls.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The trailing path segment, used to name the local file the URL is saved as.
    pub fn file_name(&self) -> &str {
        self.0
            .rsplit(['/', '\\'])
            .next()
            .filter(|segment| !segment.is_empty())
            .unwrap_or("archive.bin")
    }
}

impl From<String> for Url {
    fn from(value: String) -> Self {
        Url(value)
    }
}

impl fmt::Display for Url {
    /// Render the URL verbatim.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
