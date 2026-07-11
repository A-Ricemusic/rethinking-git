use rgit_objects::{
    AnyObject, CanonicalLimits, DecodeObjectError, ObjectKind, ReferenceRole, Value,
};

fn oid(seed: u8) -> Value {
    let mut bytes = vec![0, 0x12, 32];
    bytes.extend([seed; 32]);
    Value::Bytes(bytes)
}

fn policy_ref(seed: u8) -> Value {
    Value::Map(vec![(0, Value::Bytes(vec![seed; 16])), (1, oid(seed))])
}

fn typed(kind: ObjectKind, seed: u8) -> Value {
    Value::Map(vec![(0, Value::Unsigned(kind as u64)), (1, oid(seed))])
}

fn wall_time() -> Value {
    Value::Map(vec![(0, Value::Signed(0)), (1, Value::Signed(0))])
}

fn signature(purpose: &str) -> Value {
    let purpose = match purpose {
        "line-state" => 0,
        "operation" => 1,
        "marker" => 2,
        "release" => 3,
        "policy" => 4,
        _ => u64::MAX,
    };
    Value::Map(vec![
        (0, Value::Unsigned(0)),
        (1, Value::Bytes(vec![1; 16])),
        (2, Value::Bytes(vec![2; 32])),
        (3, Value::Bytes(vec![3; 64])),
        (4, Value::Unsigned(purpose)),
    ])
}

fn decode(value: &Value) -> Result<AnyObject, DecodeObjectError> {
    AnyObject::decode(&value.encode().unwrap(), CanonicalLimits::default())
}

fn operation() -> Value {
    Value::Map(vec![
        (0, Value::Unsigned(ObjectKind::Operation as u64)),
        (1, Value::Unsigned(0)),
        (2, policy_ref(1)),
        (3, Value::Array(vec![oid(2)])),
        (4, Value::Bytes(vec![3; 16])),
        (5, Value::Bytes(vec![4; 16])),
        (6, Value::Unsigned(1)),
        (7, wall_time()),
        (
            8,
            Value::Array(vec![Value::Map(vec![
                (0, Value::Unsigned(0)),
                (1, typed(ObjectKind::Blob, 5)),
                (2, typed(ObjectKind::Snapshot, 6)),
            ])]),
        ),
        (9, Value::Array(vec![oid(7)])),
        (10, oid(8)),
        (11, oid(9)),
        (12, signature("operation")),
        (13, Value::Text("rgit-test".into())),
    ])
}

fn policy() -> Value {
    Value::Map(vec![
        (0, Value::Unsigned(ObjectKind::Policy as u64)),
        (1, Value::Unsigned(0)),
        (3, Value::Bytes(vec![1; 16])),
        (4, Value::Unsigned(0)),
        (
            6,
            Value::Array(vec![Value::Map(vec![
                (0, Value::Unsigned(0)),
                (1, Value::Bytes(vec![2])),
            ])]),
        ),
        (
            7,
            Value::Array(vec![Value::Map(vec![
                (0, Value::Unsigned(0)),
                (
                    1,
                    Value::Array(vec![Value::Unsigned(0), Value::Unsigned(8)]),
                ),
            ])]),
        ),
        (8, Value::Unsigned(0)),
        (9, Value::Unsigned(0)),
        (10, Value::Array(vec![oid(10)])),
        (11, Value::Unsigned(0)),
        (12, oid(11)),
        (13, Value::Array(vec![Value::Bytes(vec![3; 16])])),
        (14, Value::Bytes(vec![])),
        (15, Value::Array(vec![signature("policy")])),
    ])
}

fn replace(map: &mut Value, key: u64, replacement: Value) {
    let Value::Map(fields) = map else {
        panic!("test object must be a map")
    };
    fields.iter_mut().find(|(k, _)| *k == key).unwrap().1 = replacement;
}

#[test]
fn operation_actions_are_closed_and_fully_typed() {
    let valid = operation();
    decode(&valid).unwrap();

    for malformed_action in [
        Value::Map(vec![(0, Value::Unsigned(0))]),
        Value::Map(vec![
            (0, Value::Text("wrong".into())),
            (1, typed(ObjectKind::Blob, 1)),
        ]),
        Value::Map(vec![(0, Value::Unsigned(0)), (1, Value::Null)]),
        Value::Map(vec![
            (0, Value::Unsigned(0)),
            (1, typed(ObjectKind::Blob, 1)),
            (3, Value::Null),
        ]),
        Value::Map(vec![
            (0, Value::Unsigned(0)),
            (1, Value::Map(vec![(0, Value::Unsigned(99)), (1, oid(1))])),
        ]),
    ] {
        let mut value = valid.clone();
        replace(&mut value, 8, Value::Array(vec![malformed_action]));
        assert!(decode(&value).is_err());
    }

    let mut wrong_inverse = valid.clone();
    replace(&mut wrong_inverse, 9, Value::Array(vec![Value::Null]));
    assert!(decode(&wrong_inverse).is_err());

    let mut duplicated_parents = valid.clone();
    replace(
        &mut duplicated_parents,
        3,
        Value::Array(vec![oid(2), oid(2)]),
    );
    assert!(decode(&duplicated_parents).is_err());

    let mut wrong_signature_purpose = valid.clone();
    replace(
        &mut wrong_signature_purpose,
        12,
        signature("not-a-registered-purpose"),
    );
    assert!(decode(&wrong_signature_purpose).is_err());
}

#[test]
fn operation_reference_roles_preserve_before_and_after_types() {
    let object = decode(&operation()).unwrap();
    let edges = object.references().unwrap();
    for (role, expected) in [
        (ReferenceRole::ObjectPolicy, Some(ObjectKind::Policy)),
        (ReferenceRole::OperationParent, Some(ObjectKind::Operation)),
        (ReferenceRole::OperationBefore, Some(ObjectKind::Blob)),
        (ReferenceRole::OperationAfter, Some(ObjectKind::Snapshot)),
        (
            ReferenceRole::OperationInversePayload,
            Some(ObjectKind::OperationPayload),
        ),
        (
            ReferenceRole::OperationPublicEnvelope,
            Some(ObjectKind::OperationPayload),
        ),
        (
            ReferenceRole::OperationPrivatePayload,
            Some(ObjectKind::OperationPayload),
        ),
    ] {
        assert!(
            edges
                .iter()
                .any(|edge| edge.role == role && edge.expected_kind == expected),
            "missing edge {role:?}"
        );
    }
}

#[test]
fn conflict_regions_are_closed_ordered_nonempty_ranges() {
    let base = Value::Map(vec![
        (0, Value::Unsigned(ObjectKind::Conflict as u64)),
        (1, Value::Unsigned(0)),
        (2, policy_ref(1)),
        (3, typed(ObjectKind::Blob, 2)),
        (4, typed(ObjectKind::Blob, 3)),
        (5, typed(ObjectKind::Blob, 4)),
        (6, Value::Array(vec![Value::Text("file".into())])),
        (7, Value::Unsigned(0)),
        (8, Value::Text("text".into())),
        (9, Value::Text("1".into())),
        (
            10,
            Value::Array(vec![
                Value::Map(vec![(0, Value::Unsigned(1)), (1, Value::Unsigned(2))]),
                Value::Map(vec![(0, Value::Unsigned(2)), (1, Value::Unsigned(3))]),
            ]),
        ),
    ]);
    decode(&base).unwrap();

    for regions in [
        vec![Value::Map(vec![(0, Value::Unsigned(1))])],
        vec![Value::Map(vec![
            (0, Value::Unsigned(1)),
            (1, Value::Unsigned(1)),
        ])],
        vec![Value::Map(vec![
            (0, Value::Unsigned(1)),
            (1, Value::Unsigned(2)),
            (2, Value::Unsigned(3)),
        ])],
        vec![
            Value::Map(vec![(0, Value::Unsigned(2)), (1, Value::Unsigned(4))]),
            Value::Map(vec![(0, Value::Unsigned(3)), (1, Value::Unsigned(5))]),
        ],
    ] {
        let mut value = base.clone();
        replace(&mut value, 10, Value::Array(regions));
        assert!(decode(&value).is_err());
    }
}

#[test]
fn policy_validates_all_nested_memberships_and_metadata() {
    decode(&policy()).unwrap();

    for (key, malformed) in [
        (
            6,
            Value::Array(vec![Value::Map(vec![
                (0, Value::Unsigned(4)),
                (1, Value::Bytes(vec![1])),
            ])]),
        ),
        (
            6,
            Value::Array(vec![Value::Map(vec![
                (0, Value::Unsigned(0)),
                (1, Value::Bytes(vec![])),
            ])]),
        ),
        (
            7,
            Value::Array(vec![Value::Map(vec![
                (0, Value::Unsigned(1)),
                (1, Value::Array(vec![])),
            ])]),
        ),
        (
            7,
            Value::Array(vec![Value::Map(vec![
                (0, Value::Unsigned(0)),
                (
                    1,
                    Value::Array(vec![Value::Unsigned(8), Value::Unsigned(0)]),
                ),
            ])]),
        ),
        (
            7,
            Value::Array(vec![Value::Map(vec![
                (0, Value::Unsigned(0)),
                (1, Value::Array(vec![Value::Unsigned(9)])),
            ])]),
        ),
        (10, Value::Array(vec![oid(1), oid(1)])),
        (11, Value::Text("epoch".into())),
        (13, Value::Array(vec![Value::Bytes(vec![0; 15])])),
        (14, Value::Text("constraints".into())),
    ] {
        let mut value = policy();
        replace(&mut value, key, malformed);
        assert!(
            decode(&value).is_err(),
            "accepted malformed policy field {key}"
        );
    }

    let mut unknown_principal_field = policy();
    replace(
        &mut unknown_principal_field,
        6,
        Value::Array(vec![Value::Map(vec![
            (0, Value::Unsigned(0)),
            (1, Value::Bytes(vec![1])),
            (2, Value::Null),
        ])]),
    );
    assert!(decode(&unknown_principal_field).is_err());

    let mut missing_previous = policy();
    replace(&mut missing_previous, 4, Value::Unsigned(1));
    assert!(decode(&missing_previous).is_err());
}

#[test]
fn policy_reference_extraction_includes_declassification_dependencies() {
    let edges = decode(&policy()).unwrap().references().unwrap();
    assert!(edges.iter().any(|edge| {
        edge.role == ReferenceRole::PolicyDeclassificationRequirement
            && edge.expected_kind == Some(ObjectKind::PolicyDecisionEvidence)
    }));
    assert!(edges.iter().any(|edge| {
        edge.role == ReferenceRole::PolicyKeyEnvelopeSet
            && edge.expected_kind == Some(ObjectKind::KeyEnvelopeSet)
    }));
}

#[test]
fn optional_revision_ids_and_policy_refs_are_not_shape_loopholes() {
    let mut change = Value::Map(vec![
        (0, Value::Unsigned(ObjectKind::ChangeRevision as u64)),
        (1, Value::Unsigned(0)),
        (2, policy_ref(1)),
        (3, Value::Bytes(vec![1; 16])),
        (4, Value::Null),
        (5, oid(2)),
        (6, oid(3)),
        (7, oid(4)),
        (8, oid(5)),
        (9, Value::Bytes(vec![2; 16])),
        (10, Value::Unsigned(0)),
        (11, Value::Bytes(vec![3; 16])),
        (12, Value::Bytes(vec![4; 16])),
        (13, Value::Unsigned(0)),
        (14, policy_ref(2)),
        (15, policy_ref(3)),
    ]);
    assert!(decode(&change).is_err());
    replace(&mut change, 4, oid(6));
    decode(&change).unwrap();

    let mut bad_policy = policy_ref(1);
    let Value::Map(fields) = &mut bad_policy else {
        unreachable!()
    };
    fields.push((2, Value::Null));
    replace(&mut change, 14, bad_policy);
    assert!(decode(&change).is_err());
}

#[test]
fn line_previous_state_is_required_exactly_after_genesis() {
    let mut line = Value::Map(vec![
        (0, Value::Unsigned(ObjectKind::LineState as u64)),
        (1, Value::Unsigned(0)),
        (2, policy_ref(1)),
        (3, Value::Bytes(vec![1; 16])),
        (4, Value::Text("main".into())),
        (5, oid(2)),
        (6, Value::Unsigned(1)),
        (7, Value::Null),
        (8, policy_ref(2)),
        (9, policy_ref(3)),
        (10, policy_ref(4)),
        (11, policy_ref(5)),
        (12, oid(6)),
        (13, signature("line-state")),
    ]);
    assert!(decode(&line).is_err());
    replace(&mut line, 7, oid(7));
    decode(&line).unwrap();
    replace(&mut line, 6, Value::Unsigned(0));
    assert!(decode(&line).is_err());
}

#[test]
fn line_state_reference_edges_are_unique() {
    let line = Value::Map(vec![
        (0, Value::Unsigned(ObjectKind::LineState as u64)),
        (1, Value::Unsigned(0)),
        (2, policy_ref(1)),
        (3, Value::Bytes(vec![1; 16])),
        (4, Value::Text("main".into())),
        (5, oid(2)),
        (6, Value::Unsigned(0)),
        (8, policy_ref(2)),
        (9, policy_ref(3)),
        (10, policy_ref(4)),
        (11, policy_ref(5)),
        (12, oid(6)),
        (13, signature("line-state")),
    ]);
    let edges = decode(&line).unwrap().references().unwrap();

    for (index, edge) in edges.iter().enumerate() {
        assert!(
            !edges[..index].contains(edge),
            "duplicate reference edge: {edge:?}"
        );
    }
    assert_eq!(
        edges
            .iter()
            .filter(|edge| edge.role == ReferenceRole::ObjectPolicy)
            .count(),
        1
    );
}
