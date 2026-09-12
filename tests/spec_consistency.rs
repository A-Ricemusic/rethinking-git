//! Documentation checks live in the repository, not in distributable library crates.
use std::collections::BTreeSet;

#[test]
fn documented_edge_roles_match_frozen_storage_registry() {
    let spec = include_str!("../spec/sqlite-store.md");
    let documented: Vec<_> = spec
        .split("complete version-1 closed registry is:")
        .nth(1)
        .unwrap()
        .split("```text")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap()
        .split_whitespace()
        .collect();
    let source = include_str!("../crates/rgit-store/src/edge_roles.rs");
    let frozen: Vec<_> = source
        .lines()
        .filter_map(|line| {
            line.split_once(" => \"")
                .map(|(_, rest)| rest.split('"').next().unwrap())
        })
        .collect();
    assert_eq!(frozen.len(), 97);
    assert_eq!(frozen.iter().collect::<BTreeSet<_>>().len(), frozen.len());
    assert_eq!(documented, frozen);
}

#[test]
fn documented_canonical_limits_match_crate_format() {
    let normative = include_str!("../spec/canonical-encoding.md");
    let format = include_str!("../crates/rgit-objects/FORMAT.md");
    for required in [
        "1,048,576 bytes (1 MiB)",
        "16,777,216 bytes (16 MiB)",
        "262,144 bytes (256 KiB)",
        "4,194,304 bytes (4 MiB)",
        "65,536 bytes (64 KiB)",
        "1,000,000",
        "Nested container depth | 64 | 64",
    ] {
        assert!(normative.contains(required), "spec omits {required}");
    }
    for required in [
        "1 MiB encoded",
        "16 MiB encoded",
        "256 KiB byte string",
        "4 MiB byte string",
        "1,000,000 items",
        "64 nested container levels",
    ] {
        assert!(format.contains(required), "FORMAT omits {required}");
    }
}
