//! Fetching and parsing a plugin's available versions from a remote endpoint.
//!
//! A [`VersionsSource`] declares a JSON URL plus extraction rules. [`fetch`]
//! pulls the body over HTTP and yields the version strings it encodes, so a
//! plugin can self-declare its version line instead of hardcoding a list.

use std::io::Read;

use crate::Result;
use crate::error::Error;

use super::VersionsSource;

impl VersionsSource {
    /// Fetch the endpoint URL and extract the encoded version strings.
    ///
    /// Performs the network I/O; parsing is delegated to [`Self::parse`].
    pub fn fetch(&self) -> Result<Vec<String>> {
        let body = get_body(&self.url)?;
        let text =
            std::str::from_utf8(&body).map_err(|error| malformed(&self.url, error.to_string()))?;
        self.parse(text)
    }

    /// Extract version strings from the response body.
    ///
    /// Accepts an array of objects (`field` names the key to read), an array of
    /// bare strings, or an object whose keys are the versions (`field` names
    /// the nested version map; without `field`, the top-level keys are used).
    /// Every version has `strip` removed from the front and must parse as
    /// SemVer; `stable_only` drops any value carrying a pre-release segment.
    /// Non-conforming entries are skipped. Fails when the body yields no usable
    /// version at all.
    pub fn parse(&self, body: &str) -> Result<Vec<String>> {
        let json: serde_json::Value = serde_json::from_str(body).map_err(|error| {
            malformed(
                &self.url,
                format!("response body is not valid JSON: {error}"),
            )
        })?;
        let versions: Vec<String> = self
            .candidates(&json)
            .into_iter()
            .filter_map(|raw| self.normalize(&raw))
            .collect();
        if versions.is_empty() {
            return Err(Error::Other(format!(
                "versions source {} yielded no supported versions",
                self.url
            )));
        }
        Ok(versions)
    }

    /// Produce the candidate raw version strings encoded by the JSON value.
    fn candidates(&self, json: &serde_json::Value) -> Vec<String> {
        match json {
            serde_json::Value::Array(elements) => match &self.field {
                Some(field) => elements
                    .iter()
                    .filter_map(|element| {
                        element
                            .get(field)
                            .and_then(|value| value.as_str())
                            .map(str::to_owned)
                    })
                    .collect(),
                None => elements
                    .iter()
                    .filter_map(|element| element.as_str().map(str::to_owned))
                    .collect(),
            },
            serde_json::Value::Object(map) => match &self.field {
                Some(field) => map
                    .get(field)
                    .and_then(serde_json::Value::as_object)
                    .map(|versions| versions.keys().cloned().collect())
                    .unwrap_or_default(),
                None => map.keys().cloned().collect(),
            },
            _ => Vec::new(),
        }
    }

    /// Strip the prefix and validate a candidate, returning its usable form.
    fn normalize(&self, raw: &str) -> Option<String> {
        let stripped = self.stripped(raw);
        let version = stripped.parse::<semver::Version>().ok()?;
        if self.stable_only && !version.pre.is_empty() {
            return None;
        }
        Some(stripped)
    }

    /// Remove the declared prefix when present, leaving the version otherwise.
    fn stripped(&self, raw: &str) -> String {
        match &self.strip {
            Some(prefix) => match raw.strip_prefix(prefix) {
                Some(rest) => rest.to_string(),
                None => raw.to_string(),
            },
            None => raw.to_string(),
        }
    }
}

/// Describe a version-source fetch failure for `url`.
fn malformed(url: &str, detail: String) -> Error {
    Error::Other(format!("versions source {url}: {detail}"))
}

/// Fetch a small URL (a version listing) into bytes over HTTP.
fn get_body(url: &str) -> Result<Vec<u8>> {
    let mut reader = ureq::get(url)
        .call()
        .map_err(|error| crate::Error::Network(format!("failed to fetch {url}: {error}")))?
        .into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| crate::Error::Network(format!("failed to read {url}: {error}")))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests;
