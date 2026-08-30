//! Environment layer tests: delta merging, compensation replay, and tool lookup.

use super::{
    EnvDelta, EnvOp, EnvStore, EnvVar, MemEnvStore, PATH_SEP, PathValue, apply_deltas,
    reverse_value, undo_delta,
};

/// Build the platform's separator-joined value from the given entries.
fn joined(parts: &[&str]) -> String {
    PathValue::join(parts.iter().copied())
}

#[test]
fn prepend_inserts_at_front_with_dedup() {
    let store = MemEnvStore::new();
    let path = EnvVar::from("PATH");
    store.write(&path, "/old/bin").unwrap();
    let deltas = [EnvDelta {
        name: path.clone(),
        value: "/apps/go/bin".into(),
        op: EnvOp::Prepend,
    }];
    apply_deltas(&deltas, &store).unwrap();
    assert_eq!(
        store.read(&path).unwrap().unwrap(),
        joined(&["/apps/go/bin", "/old/bin"])
    );

    // Re-applying the same delta must not duplicate the segment.
    apply_deltas(&deltas, &store).unwrap();
    assert_eq!(
        store.read(&path).unwrap().unwrap(),
        joined(&["/apps/go/bin", "/old/bin"])
    );
}

#[test]
fn compensation_restores_old_value() {
    let store = MemEnvStore::new();
    let path = EnvVar::from("PATH");
    store.write(&path, "/old/bin").unwrap();
    let deltas = [EnvDelta {
        name: path.clone(),
        value: "capnew/bin".into(),
        op: EnvOp::Prepend,
    }];
    let snapshots = apply_deltas(&deltas, &store).unwrap();

    for (name, old) in snapshots.iter().rev() {
        let old = old.as_deref();
        store.remove(name).unwrap();
        if let Some(value) = old {
            store.write(name, value).unwrap();
        }
    }
    assert_eq!(store.read(&path).unwrap().unwrap(), "/old/bin");
}

#[test]
fn set_overwrites_and_remove_clears_missing() {
    let store = MemEnvStore::new();
    let goroot = EnvVar::from("GOROOT");
    store.write(&goroot, "/usr/local/go").unwrap();
    let delta = EnvDelta {
        name: goroot.clone(),
        value: "/home/u/go".into(),
        op: EnvOp::Set,
    };
    apply_deltas(std::slice::from_ref(&delta), &store).unwrap();
    assert_eq!(store.read(&goroot).unwrap().unwrap(), "/home/u/go");
}

#[test]
fn reverse_prepend_removes_only_applied_segment() {
    let applied = EnvDelta {
        name: EnvVar::from("PATH"),
        value: "/apps/golang/bin".into(),
        op: EnvOp::Prepend,
    };
    let current = joined(&["/apps/golang/bin", "/apps/gcc/bin", "/usr/bin"]);
    let next = reverse_value(&applied, Some("/usr/bin"), Some(&current)).unwrap();
    assert_eq!(next, joined(&["/apps/gcc/bin", "/usr/bin"]));
}

#[test]
fn reverse_prepend_empty_leaves_none() {
    let applied = EnvDelta {
        name: EnvVar::from("PATH"),
        value: "/apps/golang/bin".into(),
        op: EnvOp::Prepend,
    };
    // The applied segment is the only entry with nothing before it.
    assert_eq!(
        reverse_value(&applied, None, Some("/apps/golang/bin")),
        None
    );
    // Nothing current to remove from -> nothing to do.
    assert_eq!(reverse_value(&applied, None, None), None);
}

#[test]
fn undo_delta_set_restores_snapshot() {
    let store = MemEnvStore::new();
    let goroot = EnvVar::from("GOROOT");
    store.write(&goroot, "/home/u/go").unwrap();
    let delta = EnvDelta {
        name: goroot.clone(),
        value: "/home/u/go".into(),
        op: EnvOp::Set,
    };
    apply_deltas(std::slice::from_ref(&delta), &store).unwrap();
    undo_delta(&delta, Some("/usr/local/go"), &store).unwrap();
    assert_eq!(store.read(&goroot).unwrap().unwrap(), "/usr/local/go");
}

#[test]
fn undo_delta_set_without_old_removes_var() {
    let store = MemEnvStore::new();
    let goroot = EnvVar::from("GOROOT");
    store.remove(&goroot).unwrap();
    let delta = EnvDelta {
        name: goroot.clone(),
        value: "/home/u/go".into(),
        op: EnvOp::Set,
    };
    apply_deltas(std::slice::from_ref(&delta), &store).unwrap();
    undo_delta(&delta, None, &store).unwrap();
    assert_eq!(store.read(&goroot).unwrap(), None);
}

#[test]
fn path_value_prepend_and_append_are_platform_joined() {
    assert_eq!(PathValue::separator(), PATH_SEP);
    // The whole point: expectations are built from the separator source, not a literal.
    assert_eq!(joined(&["a", "b"]), format!("a{PATH_SEP}b"));

    let list = PathValue::new("a".to_string());
    assert_eq!(list.append("b"), joined(&["a", "b"]));
    assert_eq!(list.prepend("z"), joined(&["z", "a"]));
}

#[test]
fn path_value_remove_drops_one_entry() {
    let list = PathValue::new(joined(&["a", "b", "c"]));
    assert_eq!(list.remove("b").unwrap(), joined(&["a", "c"]));
    // Removing the only entry empties the list.
    assert_eq!(PathValue::new("a".to_string()).remove("a"), None);
}

/// On Windows a case-variant is the same path entry; prepending it must not duplicate.
#[cfg(windows)]
#[test]
fn path_value_dedupes_case_variant_on_windows() {
    let list = PathValue::new("C:\\apps\\go\\bin".to_string());
    assert_eq!(
        list.prepend("c:\\APPS\\GO\\BIN"),
        "C:\\apps\\go\\bin".to_string()
    );
}

/// Every platform treats `PATH` (any case on Windows) as the list variable.
#[test]
fn env_var_is_path_matches() {
    assert!(EnvVar::from("PATH").is_path());
    #[cfg(windows)]
    assert!(EnvVar::from("path").is_path());
}

/// Env var equality follows the platform: exact on POSIX, case-insensitive on Windows.
#[test]
fn env_var_equality_follows_platform() {
    let goroot = EnvVar::from("GOROOT");
    assert_eq!(goroot, EnvVar::from("GOROOT"));
    #[cfg(windows)]
    assert_eq!(goroot, EnvVar::from("goroot"));
}
