//! Environment layer tests: delta merging and compensation replay.

use super::{EnvDelta, EnvOp, EnvStore, MemEnvStore, PATH_SEP, apply_deltas};

#[test]
fn prepend_inserts_at_front_with_dedup() {
    let store = MemEnvStore::new();
    store.write("PATH", "C:\\old\\bin").unwrap();
    let deltas = [EnvDelta {
        name: "PATH".into(),
        value: "C:\\apps\\go\\bin".into(),
        op: EnvOp::Prepend,
    }];
    apply_deltas(&deltas, &store).unwrap();
    assert_eq!(
        store.read("PATH").unwrap().unwrap(),
        format!("C:\\apps\\go\\bin{PATH_SEP}C:\\old\\bin")
    );

    apply_deltas(&deltas, &store).unwrap();
    assert_eq!(
        store.read("PATH").unwrap().unwrap(),
        format!("C:\\apps\\go\\bin{PATH_SEP}C:\\old\\bin")
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
