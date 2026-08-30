//! Template placeholder expansion shared by every install method.
//!
//! Plugins spell install destinations as templates (`{base}`, `{version}`,
//! `{user}`) so a manifest never hardcodes a machine-specific path. All three
//! substitution sites (command invocations, installer args, environment values)
//! route through this module, so the placeholder rules live in exactly one place.

use std::path::Path;

use crate::Version;

/// Substitute `{base}`, `{version}`, and `{user}` placeholders in `template`.
///
/// `base` becomes the install directory for this run, `{version}` the pinned
/// version (or `latest`), and `{user}` the current user's home directory.
pub(crate) fn expand_template(template: &str, base: &Path, version: &Version) -> String {
    let base_str = base.to_string_lossy();
    let user_home = crate::env::user_home();
    template
        .replace("{base}", &base_str)
        .replace("{version}", &version.to_string())
        .replace("{user}", &user_home)
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
