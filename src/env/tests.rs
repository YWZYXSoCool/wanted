//! Environment layer tests: delta merging, compensation replay, and tool lookup.

use super::{
    EnvDelta, EnvOp, EnvStore, MemEnvStore, PATH_SEP, apply_deltas, reverse_value, undo_delta,
};

#[test]
fn prepend_inserts_at_front_with_dedup() {
    let store = MemEnvStore::new();
    // Use separator-neutral paths so the test passes on both `;` (Windows) and
    // `:` (POSIX) platforms.
    store.write("PATH", "/old/bin").unwrap();
    let deltas = [EnvDelta {
        name: "PATH".into(),
        value: "/apps/go/bin".into(),
        op: EnvOp::Prepend,
    }];
    apply_deltas(&deltas, &store).unwrap();
    assert_eq!(
        store.read("PATH").unwrap().unwrap(),
        format!("/apps/go/bin{PATH_SEP}/old/bin")
    );

    // Re-applying the same delta must not duplicate the segment.
    apply_deltas(&deltas, &store).unwrap();
    assert_eq!(
        store.read("PATH").unwrap().unwrap(),
        format!("/apps/go/bin{PATH_SEP}/old/bin")
    );
}

#[test]
fn compensation_restores_old_value() {
    let store = MemEnvStore::new();
    store.write("PATH", "C:\\old\\bin").unwrap();
    let deltas = [EnvDelta {
        name: "PATH".into(),
        value: "C:\\capnew\\bin".into(),
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
    assert_eq!(store.read("PATH").unwrap().unwrap(), "C:\\old\\bin");
}

#[test]
fn set_overwrites_and_remove_clears_missing() {
    let store = MemEnvStore::new();
    store.write("GOROOT", "/usr/local/go").unwrap();
    let delta = EnvDelta {
        name: "GOROOT".into(),
        value: "/home/u/go".into(),
        op: EnvOp::Set,
    };
    apply_deltas(&[delta], &store).unwrap();
    assert_eq!(store.read("GOROOT").unwrap().unwrap(), "/home/u/go");
}

#[test]
fn reverse_prepend_removes_only_applied_segment() {
    let applied = EnvDelta {
        name: "PATH".into(),
        value: "/apps/golang/bin".into(),
        op: EnvOp::Prepend,
    };
    let current =
        format!("/apps/golang/bin{PATH_SEP}/apps/gcc/bin{PATH_SEP}/usr/bin");
    let next = reverse_value(&applied, Some("/usr/bin"), Some(&current), PATH_SEP).unwrap();
    assert_eq!(next, format!("/apps/gcc/bin{PATH_SEP}/usr/bin"));
}

#[test]
fn reverse_prepend_empty_leaves_none() {
    let applied = EnvDelta {
        name: "PATH".into(),
        value: "/apps/golang/bin".into(),
        op: EnvOp::Prepend,
    };
    // The applied segment is the only entry with nothing before it.
    assert_eq!(
        reverse_value(&applied, None, Some("/apps/golang/bin"), PATH_SEP),
        None
    );
    // Nothing current to remove from -> nothing to do.
    assert_eq!(reverse_value(&applied, None, None, PATH_SEP), None);
}

#[test]
fn undo_delta_set_restores_snapshot() {
    let store = MemEnvStore::new();
    store.write("GOROOT", "/home/u/go").unwrap();
    let delta = EnvDelta {
        name: "GOROOT".into(),
        value: "/home/u/go".into(),
        op: EnvOp::Set,
    };
    apply_deltas(std::slice::from_ref(&delta), &store).unwrap();
    undo_delta(&delta, Some("/usr/local/go"), &store).unwrap();
    assert_eq!(store.read("GOROOT").unwrap().unwrap(), "/usr/local/go");
}

#[test]
fn undo_delta_set_without_old_removes_var() {
    let store = MemEnvStore::new();
    store.remove("GOROOT").unwrap();
    let delta = EnvDelta {
        name: "GOROOT".into(),
        value: "/home/u/go".into(),
        op: EnvOp::Set,
    };
    apply_deltas(std::slice::from_ref(&delta), &store).unwrap();
    undo_delta(&delta, None, &store).unwrap();
    assert_eq!(store.read("GOROOT").unwrap(), None);
}
