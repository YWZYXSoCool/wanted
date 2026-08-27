//! Platform triplet assembly tests.

use super::Target;

#[test]
fn windows_triplet_is_pc_windows_env() {
    let target = Target {
        arch: "x86_64",
        os: "windows",
        env: "msvc",
    };
    assert_eq!(target.triplet(), "x86_64-pc-windows-msvc");
}

#[test]
fn macos_triplet_is_apple_darwin() {
    let target = Target {
        arch: "aarch64",
        os: "macos",
        env: "",
    };
    assert_eq!(target.triplet(), "aarch64-apple-darwin");
}
