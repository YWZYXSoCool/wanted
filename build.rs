//! Regenerates `schema/plugin.schema.json` from the raw manifest types.
//!
//! `build.rs` runs as a separate crate and cannot import the library, so it
//! `include!`s the same [`raw.rs`](src/plugin/raw.rs) the library parses with.
//! Because both consumers share one source file, the generated JSON Schema can
//! never drift from what [`Manifest::parse`](wanted::plugin::Manifest::parse)
//! actually accepts.

use std::env;
use std::fs;
use std::path::PathBuf;

include!("src/plugin/raw.rs");

fn main() {
    // Re-run only when the manifest shapes change (or the build script itself).
    println!("cargo:rerun-if-changed=src/plugin/raw.rs");
    println!("cargo:rerun-if-changed=build.rs");

    let schema = schemars::schema_for!(RawManifest);
    let json = serde_json::to_string_pretty(&schema).expect("schema serializes to JSON");

    let out = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"))
        .join("schema/plugin.schema.json");
    fs::create_dir_all(out.parent().expect("schema dir")).expect("create schema dir");
    fs::write(&out, json + "\n").expect("write schema file");
}
