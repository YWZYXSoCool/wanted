//! Template placeholder expansion shared by every install method.
//!
//! Plugins spell install destinations as templates (`{base}`, `{version}`,
//! `{user}`) so a manifest never hardcodes a machine-specific path. All three
//! substitution sites (command invocations, installer args, environment values)
//! route through this module, so the placeholder rules live in exactly one place.

use std::path::Path;

use crate::Version;

/// Substitute `{base}`, `{version}`, `{date}`, and `{user}` placeholders.
///
/// `base` becomes the install directory for this run, `{version}` the pinned
/// version (or `latest`), `{date}` that version's SemVer build-metadata suffix
/// (empty when absent, e.g. the `20260825` in `3.13.0+20260825`), and `{user}`
/// the current user's home directory.
pub(crate) fn expand_template(template: &str, base: &Path, version: &Version) -> String {
    let base_str = base.to_string_lossy();
    let user_home = crate::env::user_home();
    template
        .replace("{base}", &base_str)
        .replace("{version}", &version.to_string())
        .replace("{date}", &build_date(version))
        .replace("{user}", &user_home)
}

/// Substitute the version-only placeholders (`{version}`, `{date}`) for a URL
/// template. Asset URLs carry no `{base}`/`{user}` and are expanded directly,
/// so this stays separate from [`expand_template`].
pub(crate) fn expand_url(template: &str, version: &Version) -> String {
    template
        .replace("{version}", &version.to_string())
        .replace("{date}", &build_date(version))
}

/// The SemVer build-metadata tail of a pinned version (`20260825` for
/// `3.13.0+20260825`); empty for `latest` or a version with no build suffix.
fn build_date(version: &Version) -> String {
    match version {
        Version::Pinned(value) => value.build.as_str().to_string(),
        Version::Latest => String::new(),
    }
}

/// Resolve an environment-value template, joining relative paths under `base` so
/// the tool writes absolute paths.
///
/// `$`-prefixed and already-absolute templates pass through untouched; an empty
/// or `.` value resolves to `base` itself. Placeholder substitution reuses
/// [`expand_template`]; only the path placement rule lives here.
pub(crate) fn resolve_template(template: &str, base: &Path, version: &Version) -> String {
    let substituted = expand_template(template, base, version);
    let value_path = Path::new(&substituted);
    if template.starts_with('$') || value_path.is_absolute() {
        substituted
    } else if value_path == Path::new(".") || value_path.as_os_str().is_empty() {
        base.to_string_lossy().into_owned()
    } else {
        base.join(value_path).to_string_lossy().into_owned()
    }
}
