//! Platform target identification.

/// The currently running platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    arch: &'static str,
    os: &'static str,
    env: &'static str,
}

impl Target {
    /// The current compile target's platform.
    #[inline]
    pub const fn current() -> Target {
        Target {
            arch: std::env::consts::ARCH,
            os: std::env::consts::OS,
            env: current_env(),
        }
    }

    /// Explicitly construct a target, for cross-module tests and tooling.
    #[inline]
    pub const fn parts(arch: &'static str, os: &'static str, env: &'static str) -> Target {
        Target { arch, os, env }
    }

    /// Rust-style triplet, e.g. `x86_64-pc-windows-msvc`.
    pub fn triplet(&self) -> String {
        match self.os {
            "windows" => format!("{}-pc-windows-{}", self.arch, self.env),
            "linux" => format!("{}-unknown-linux-{}", self.arch, self.env),
            "macos" => format!("{}-apple-darwin", self.arch),
            other => format!("{}-{}", self.arch, other),
        }
    }
}

/// Resolve the current target ABI (`msvc` / `gnu` / `musl`; unknown is `""`).
#[inline]
const fn current_env() -> &'static str {
    if cfg!(target_env = "msvc") {
        "msvc"
    } else if cfg!(target_env = "gnu") {
        "gnu"
    } else if cfg!(target_env = "musl") {
        "musl"
    } else {
        ""
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.triplet())
    }
}

#[cfg(test)]
mod tests;
