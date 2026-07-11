use std::str::FromStr;

use rgit_graph::{
    ConflictKind, DiffError, DiffInput, GenerationError, GraphError, GraphIndex, GraphNode,
    MergeError, OpaqueEntry, OpaqueEntryId, TreeEntry, TreeEntryError, diff, merge_three_way,
    next_generation,
};
use rgit_objects::{
    FileMode, ManifestTarget, ObjectId, PathSegment, PolicyId, PolicyRef, PortablePath,
};

const IDS: [&str; 5] = [
    "rg0_00921kajuid6jd5b9p73g59985kbon5h46aoqnvq5th8i39aou0omi61",
    "rg0_00f21v5optb32hdhca56joiu79rlq38hpste835bcf63nuaau6vsncb6",
    "rg0_0092127unouvh9msrlf8d2msnf7g5md3j6ao1lcikms4rm1jvlo8l77k",
    "rg0_00f21tnkdgivdseg0srt8i316psk1g2fev4vhabbvercq2pl6qvk49ja",
    "rg0_009211ieitnnoghofvar9fug9nqhcu70uhg8v51kafqfsjnaevkul1mh",
];

fn id(index: usize) -> ObjectId {
    ObjectId::from_str(IDS[index]).expect("test vector object ID is valid")
}

fn path(value: &str) -> PortablePath {
    PortablePath::new(
        value
            .split('/')
            .map(|segment| PathSegment::new_portable(segment).expect("test path is portable"))
            .collect(),
    )
    .expect("test path is valid")
}

fn policy(number: u8, version: usize) -> PolicyRef {
    PolicyRef {
        policy_id: PolicyId::from_bytes([number; 16]),
        version: id(version),
    }
}

fn entry(name: &str, content: usize, policy_ref: PolicyRef) -> TreeEntry {
    TreeEntry::new(
        path(name),
        ManifestTarget::File {
            blob: id(content),
            mode: FileMode::Regular,
        },
        policy_ref,
    )
    .expect("test entry is valid")
}

fn visible(entries: Vec<TreeEntry>) -> DiffInput {
    DiffInput::new(entries, []).expect("test diff input is unique")
}

#[test]
fn diff_add_modify_delete_table_is_deterministic() {
    struct Case {
        previous: Vec<TreeEntry>,
        current: Vec<TreeEntry>,
        added: Vec<&'static str>,
        modified: Vec<&'static str>,
        deleted: Vec<&'static str>,
    }

    let public = policy(1, 0);
    let cases = [
        Case {
            previous: vec![],
            current: vec![entry("z-added", 1, public.clone())],
            added: vec!["z-added"],
            modified: vec![],
            deleted: vec![],
        },
        Case {
            previous: vec![entry("modified", 0, public.clone())],
            current: vec![entry("modified", 1, public.clone())],
            added: vec![],
            modified: vec!["modified"],
            deleted: vec![],
        },
        Case {
            previous: vec![entry("deleted", 0, public.clone())],
            current: vec![],
            added: vec![],
            modified: vec![],
            deleted: vec!["deleted"],
        },
        Case {
            previous: vec![
                entry("z-delete", 0, public.clone()),
                entry("m-modify", 0, public.clone()),
            ],
            current: vec![
                entry("a-add", 1, public.clone()),
                entry("m-modify", 2, public.clone()),
            ],
            added: vec!["a-add"],
            modified: vec!["m-modify"],
            deleted: vec!["z-delete"],
        },
    ];

    for case in cases {
        let result = diff(&visible(case.previous), &visible(case.current)).expect("diff succeeds");
        assert_eq!(
            result.added,
            case.added.into_iter().map(path).collect::<Vec<_>>()
        );
        assert_eq!(
            result.modified,
            case.modified.into_iter().map(path).collect::<Vec<_>>()
        );
        assert_eq!(
            result.deleted,
            case.deleted.into_iter().map(path).collect::<Vec<_>>()
        );
    }
}

#[test]
fn diff_treats_policy_and_manifest_reference_changes_as_modifications() {
    let original = entry("config", 0, policy(1, 0));
    let policy_changed = entry("config", 0, policy(2, 1));
    let reference_changed = TreeEntry::new(
        path("config"),
        ManifestTarget::SecretRef { secret_ref: id(2) },
        policy(1, 0),
    )
    .unwrap();

    for changed in [policy_changed, reference_changed] {
        let result = diff(&visible(vec![original.clone()]), &visible(vec![changed])).unwrap();
        assert_eq!(result.modified, vec![path("config")]);
    }
}

#[test]
fn opaque_diff_counts_changes_without_returning_hidden_identity_or_path() {
    let old = DiffInput::new(
        [],
        [
            OpaqueEntry::new(OpaqueEntryId::new([1; 32]), id(0)),
            OpaqueEntry::new(OpaqueEntryId::new([2; 32]), id(0)),
            OpaqueEntry::new(OpaqueEntryId::new([3; 32]), id(0)),
        ],
    )
    .unwrap();
    let new = DiffInput::new(
        [],
        [
            OpaqueEntry::new(OpaqueEntryId::new([2; 32]), id(1)),
            OpaqueEntry::new(OpaqueEntryId::new([3; 32]), id(0)),
            OpaqueEntry::new(OpaqueEntryId::new([4; 32]), id(0)),
        ],
    )
    .unwrap();

    let result = diff(&old, &new).unwrap();
    assert_eq!(result.opaque_changed, 3);
    assert!(result.added.is_empty());
    assert!(result.modified.is_empty());
    assert!(result.deleted.is_empty());
    assert_eq!(
        format!("{:?}", OpaqueEntryId::new([7; 32])),
        "OpaqueEntryId(..)"
    );
    let debug = format!("{new:?}");
    assert!(debug.contains("opaque_entries: 3"));
    assert!(!debug.contains(IDS[0]));
}

#[test]
fn diff_rejects_ambiguous_duplicate_visible_and_opaque_inputs() {
    let duplicate = entry("same", 0, policy(1, 0));
    assert_eq!(
        DiffInput::new([duplicate.clone(), duplicate], []),
        Err(DiffError::DuplicateVisiblePath)
    );
    assert_eq!(
        DiffInput::new(
            [],
            [
                OpaqueEntry::new(OpaqueEntryId::new([1; 32]), id(0)),
                OpaqueEntry::new(OpaqueEntryId::new([1; 32]), id(1)),
            ],
        ),
        Err(DiffError::DuplicateOpaqueIdentity)
    );
}

#[test]
fn diff_rejects_portable_sibling_and_ancestor_collisions_deterministically() {
    let public = policy(1, 0);
    let cases = [
        ("Foo", "foo"),
        ("Dir/left", "dir/right"),
        ("parent", "parent/child"),
        ("Parent", "parent/child"),
    ];

    for (first, second) in cases {
        let expected = Err(DiffError::PortablePathCollision {
            first: path(first),
            second: path(second),
        });
        assert_eq!(
            DiffInput::new(
                [
                    entry(first, 0, public.clone()),
                    entry(second, 1, public.clone()),
                ],
                [],
            ),
            expected
        );
        assert_eq!(
            DiffInput::new(
                [
                    entry(second, 1, public.clone()),
                    entry(first, 0, public.clone()),
                ],
                [],
            ),
            expected
        );
    }

    DiffInput::new(
        [
            entry("left/Foo", 0, public.clone()),
            entry("right/foo", 1, public.clone()),
            entry("same/left", 2, public.clone()),
            entry("same/right", 3, public),
        ],
        [],
    )
    .expect("fold-equivalent names in distinct directories are legal");
}

#[test]
fn merge_combines_independent_changes_in_path_order() {
    let public = policy(1, 0);
    let base = vec![entry("app", 0, public.clone())];
    let line = vec![
        entry("line", 1, public.clone()),
        entry("app", 0, public.clone()),
    ];
    let incoming = vec![
        entry("feature", 2, public.clone()),
        entry("app", 3, public.clone()),
    ];

    let plan = merge_three_way(base, line, incoming).unwrap();
    assert!(plan.conflicts.is_empty());
    assert_eq!(
        plan.merged_entries
            .iter()
            .map(TreeEntry::path)
            .cloned()
            .collect::<Vec<_>>(),
        vec![path("app"), path("feature"), path("line")]
    );
}

#[test]
fn merge_conflict_kind_table_covers_add_modify_and_delete() {
    let public = policy(1, 0);
    let cases = [
        (
            vec![],
            vec![entry("file", 1, public.clone())],
            vec![entry("file", 2, public.clone())],
            ConflictKind::AddAdd,
        ),
        (
            vec![entry("file", 0, public.clone())],
            vec![entry("file", 1, public.clone())],
            vec![entry("file", 2, public.clone())],
            ConflictKind::BothModified,
        ),
        (
            vec![entry("file", 0, public.clone())],
            vec![],
            vec![entry("file", 2, public.clone())],
            ConflictKind::DeleteModify,
        ),
    ];

    for (base, line, incoming, kind) in cases {
        let plan = merge_three_way(base, line, incoming).unwrap();
        assert!(plan.merged_entries.is_empty());
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, kind);
    }
}

#[test]
fn merge_preserves_selected_entries_and_every_conflict_policy_reference() {
    let base_policy = policy(1, 0);
    let line_policy = policy(2, 1);
    let incoming_policy = policy(3, 2);
    let unchanged = entry("unchanged", 4, base_policy.clone());
    let line = entry("conflict", 1, line_policy.clone());
    let incoming = entry("conflict", 2, incoming_policy.clone());
    let base = entry("conflict", 0, base_policy.clone());

    let plan = merge_three_way(
        vec![base.clone(), unchanged.clone()],
        vec![line.clone(), unchanged.clone()],
        vec![incoming.clone(), unchanged.clone()],
    )
    .unwrap();

    assert_eq!(plan.merged_entries, vec![unchanged]);
    let conflict = &plan.conflicts[0];
    assert_eq!(
        conflict.base.as_ref().unwrap().target,
        base.target().clone()
    );
    assert_eq!(
        conflict.line.as_ref().unwrap().target,
        line.target().clone()
    );
    assert_eq!(
        conflict.incoming.as_ref().unwrap().target,
        incoming.target().clone()
    );
    assert_eq!(
        conflict.source_policy_refs,
        vec![line_policy, incoming_policy, base_policy]
    );
}

#[test]
fn tree_and_merge_reject_ambiguous_paths() {
    assert_eq!(
        TreeEntry::new(
            PortablePath::new(vec![]).unwrap(),
            ManifestTarget::File {
                blob: id(0),
                mode: FileMode::Regular,
            },
            policy(1, 0),
        ),
        Err(TreeEntryError::EmptyPath)
    );

    let duplicate = entry("same", 0, policy(1, 0));
    assert_eq!(
        merge_three_way(
            [duplicate.clone(), duplicate],
            std::iter::empty(),
            std::iter::empty(),
        ),
        Err(MergeError::DuplicatePath { input: "base" })
    );
}

#[test]
fn merge_rejects_non_materializable_inputs_and_cross_branch_results() {
    let public = policy(1, 0);
    let input_collision = merge_three_way(
        [
            entry("Dir/a", 0, public.clone()),
            entry("dir/b", 1, public.clone()),
        ],
        [],
        [],
    );
    assert_eq!(
        input_collision,
        Err(MergeError::PortablePathCollision {
            input: "base",
            first: path("Dir/a"),
            second: path("dir/b"),
        })
    );

    for (line_name, incoming_name) in [("Foo", "foo"), ("Parent", "parent/child")] {
        let expected = Err(MergeError::PortablePathCollision {
            input: "merged result",
            first: path(line_name),
            second: path(incoming_name),
        });
        assert_eq!(
            merge_three_way(
                [],
                [entry(line_name, 0, public.clone())],
                [entry(incoming_name, 1, public.clone())],
            ),
            expected
        );
        assert_eq!(
            merge_three_way(
                [],
                [entry(incoming_name, 1, public.clone())],
                [entry(line_name, 0, public.clone())],
            ),
            expected
        );
    }

    let legal = merge_three_way(
        [],
        [entry("left/Foo", 0, public.clone())],
        [entry("right/foo", 1, public)],
    )
    .expect("equivalent names in different directories remain legal");
    assert_eq!(legal.merged_entries.len(), 2);
}

#[test]
fn merge_obeys_identity_symmetry_and_path_accounting_invariants() {
    let public = policy(1, 0);
    let base = vec![
        entry("kept", 0, public.clone()),
        entry("changed", 0, public.clone()),
        entry("deleted", 0, public.clone()),
    ];
    let changed = vec![
        entry("added", 1, public.clone()),
        entry("changed", 1, public.clone()),
        entry("kept", 0, public.clone()),
    ];

    let identity = merge_three_way(base.clone(), base.clone(), changed.clone()).unwrap();
    assert!(identity.conflicts.is_empty());
    assert_eq!(identity.merged_entries, changed);

    let line = vec![
        entry("kept", 0, public.clone()),
        entry("changed", 1, public.clone()),
        entry("line-only", 2, public.clone()),
    ];
    let incoming = vec![
        entry("kept", 0, public.clone()),
        entry("changed", 2, public.clone()),
        entry("incoming-only", 3, public),
    ];
    let forward = merge_three_way(base.clone(), line.clone(), incoming.clone()).unwrap();
    let reverse = merge_three_way(base, incoming, line).unwrap();
    assert_eq!(forward.merged_entries, reverse.merged_entries);
    assert_eq!(
        forward
            .conflicts
            .iter()
            .map(|conflict| (&conflict.path, conflict.kind))
            .collect::<Vec<_>>(),
        reverse
            .conflicts
            .iter()
            .map(|conflict| (&conflict.path, conflict.kind))
            .collect::<Vec<_>>()
    );

    let accounted = forward
        .merged_entries
        .iter()
        .map(TreeEntry::path)
        .chain(forward.conflicts.iter().map(|conflict| &conflict.path))
        .map(|path| format!("{path:?}"))
        .collect::<std::collections::BTreeSet<_>>();
    for expected in ["kept", "changed", "line-only", "incoming-only"] {
        assert!(accounted.contains(&format!("{:?}", path(expected))));
    }
}

#[test]
fn graph_diamond_has_max_parent_generations_and_deterministic_order() {
    let graph = GraphIndex::build([
        GraphNode::new("merge", vec!["right", "left"]),
        GraphNode::new("right", vec!["root"]),
        GraphNode::new("root", vec![]),
        GraphNode::new("left", vec!["root"]),
    ])
    .unwrap();

    assert_eq!(graph.generation(&"root"), Ok(0));
    assert_eq!(graph.generation(&"left"), Ok(1));
    assert_eq!(graph.generation(&"right"), Ok(1));
    assert_eq!(graph.generation(&"merge"), Ok(2));
    assert_eq!(
        graph.topological_order(),
        &["root", "left", "right", "merge"]
    );
    assert_eq!(
        graph
            .ancestors(&"merge")
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        vec!["left", "right", "root"]
    );
    assert_eq!(graph.is_reachable(&"root", &"merge"), Ok(true));
    assert_eq!(graph.is_reachable(&"left", &"right"), Ok(false));
    assert_eq!(graph.is_reachable(&"merge", &"merge"), Ok(true));
}

#[test]
fn graph_rejects_cycles_missing_parents_and_generation_overflow() {
    assert_eq!(
        GraphIndex::build([
            GraphNode::new("a", vec!["b"]),
            GraphNode::new("b", vec!["a"]),
        ]),
        Err(GraphError::Cycle {
            cycle_nodes: vec!["a", "b"]
        })
    );
    assert_eq!(
        GraphIndex::build([GraphNode::new("child", vec!["missing"])]),
        Err(GraphError::MissingParent {
            node: "child",
            parent: "missing"
        })
    );
    assert_eq!(next_generation([u64::MAX]), Err(GenerationError::Overflow));
    assert_eq!(next_generation([2, 9, 4]), Ok(10));
    assert_eq!(next_generation([]), Ok(0));
}

#[test]
fn cycle_diagnostic_excludes_acyclic_nodes_blocked_behind_the_cycle() {
    assert_eq!(
        GraphIndex::build([
            GraphNode::new("cycle-a", vec!["cycle-b"]),
            GraphNode::new("cycle-b", vec!["cycle-a"]),
            GraphNode::new("blocked-child", vec!["cycle-a"]),
            GraphNode::new("blocked-grandchild", vec!["blocked-child"]),
            GraphNode::new("independent", vec![]),
        ]),
        Err(GraphError::Cycle {
            cycle_nodes: vec!["cycle-a", "cycle-b"]
        })
    );
    assert_eq!(
        GraphIndex::build([GraphNode::new("self", vec!["self"])]),
        Err(GraphError::Cycle {
            cycle_nodes: vec!["self"]
        })
    );
}

#[test]
fn graph_normalizes_repeated_edges_but_rejects_duplicate_nodes() {
    let graph = GraphIndex::build([
        GraphNode::new("root", vec![]),
        GraphNode::new("child", vec!["root", "root"]),
    ])
    .unwrap();
    assert_eq!(graph.generation(&"child"), Ok(1));

    assert_eq!(
        GraphIndex::build([
            GraphNode::new("same", vec![]),
            GraphNode::new("same", vec![]),
        ]),
        Err(GraphError::DuplicateNode { node: "same" })
    );
}
