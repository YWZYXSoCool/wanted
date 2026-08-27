use super::*;

#[test]
fn local_path_wins_when_file_exists() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("golang.toml");
    std::fs::write(&manifest, "[meta]\nname = \"go\"\n").unwrap();
    let source = resolve_add_source(manifest.to_str().unwrap(), None);
    assert_eq!(source, PluginSource::Local(manifest));
}

#[test]
fn missing_path_defaults_to_registry() {
    let source = resolve_add_source("golang", None);
    assert_eq!(
        source,
        PluginSource::Registry {
            name: "golang".into(),
            url: format!("{}/golang.toml", DEFAULT_REGISTRY),
        }
    );
}

#[test]
fn registry_override_used_when_given() {
    let source = resolve_add_source("llvm", Some("https://example.com/registry"));
    assert_eq!(
        source,
        PluginSource::Registry {
            name: "llvm".into(),
            url: "https://example.com/registry/llvm.toml".into(),
        }
    );
}
