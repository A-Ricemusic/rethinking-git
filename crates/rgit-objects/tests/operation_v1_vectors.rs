use rgit_objects::{
    ActorId, AnyObject, CanonicalLimits, CanonicalObject, DeviceId, HashAlgorithm, ObjectId,
    ObjectKind, OperationActionV1, OperationV1, PolicyId, PolicyRef, ReferenceKey, ReferenceRole,
    Signature, SignatureAlgorithm, SignaturePurpose, SignedObject, TypedObjectRef, Value, WallTime,
    decode_canonical,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
struct Fixture {
    schema: u64,
    canonical_cbor_hex: String,
    unsigned_cbor_hex: String,
    signing_preimage_hex: String,
    sha256_binary_id_hex: String,
    sha256_text_id: String,
    blake3_binary_id_hex: String,
    blake3_text_id: String,
}

fn zero_id() -> ObjectId {
    let mut bytes = vec![0, 0x12, 32];
    bytes.extend([0; 32]);
    ObjectId::from_bytes(&bytes).unwrap()
}

fn operation() -> OperationV1 {
    let id = zero_id();
    OperationV1 {
        policy_ref: PolicyRef {
            policy_id: PolicyId::from_bytes([0x11; 16]),
            version: id.clone(),
        },
        parents: vec![id.clone()],
        actor: ActorId::from_bytes([0x22; 16]),
        device: DeviceId::from_bytes([0x33; 16]),
        logical_time: 65_536,
        wall_time: WallTime {
            utc_seconds: -1,
            offset_seconds: 3_600,
        },
        actions: vec![OperationActionV1::BoundTransition {
            key: ReferenceKey::Marker([0x55; 16]),
            before: Some(TypedObjectRef {
                kind: ObjectKind::Marker,
                id: id.clone(),
            }),
            after: TypedObjectRef {
                kind: ObjectKind::Marker,
                id: id.clone(),
            },
        }],
        inverse_payloads: vec![],
        public_envelope: None,
        private_payload: None,
        signature: Signature::new(
            SignatureAlgorithm::Ed25519,
            ActorId::from_bytes([0x44; 16]),
            [0x6b; 32],
            [0x77; 64],
            SignaturePurpose::Operation,
        )
        .unwrap(),
        client_implementation: "rgit/1".into(),
    }
}

fn map(value: &Value) -> &[(u64, Value)] {
    let Value::Map(fields) = value else {
        panic!("map")
    };
    fields
}

fn map_mut(value: &mut Value) -> &mut Vec<(u64, Value)> {
    let Value::Map(fields) = value else {
        panic!("map")
    };
    fields
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("vectors/operation-v1.json")).unwrap()
}

#[test]
fn operation_v1_signed_and_object_vectors_are_frozen() {
    let operation = operation();
    let encoded = operation.encode().unwrap();
    let unsigned = operation.unsigned_encode().unwrap();
    let signature = &operation.signature;
    let signing_preimage = operation.signing_preimage(signature).unwrap();
    let expected = fixture();
    assert_eq!(expected.schema, 1);
    assert_eq!(hex::encode(&encoded), expected.canonical_cbor_hex);
    assert_eq!(hex::encode(&unsigned), expected.unsigned_cbor_hex);
    assert_eq!(hex::encode(signing_preimage), expected.signing_preimage_hex);

    let decoded = AnyObject::decode(&encoded, CanonicalLimits::metadata()).unwrap();
    assert_eq!(decoded.decoded().kind(), ObjectKind::Operation);
    assert_eq!(decoded.decoded().schema_version(), 1);
    assert_eq!(decoded.decoded().value().encode().unwrap(), encoded);
    for (algorithm, binary, text) in [
        (
            HashAlgorithm::Sha256,
            expected.sha256_binary_id_hex,
            expected.sha256_text_id,
        ),
        (
            HashAlgorithm::Blake3_256,
            expected.blake3_binary_id_hex,
            expected.blake3_text_id,
        ),
    ] {
        let id = decoded.id(algorithm).unwrap();
        assert_eq!(hex::encode(id.to_bytes()), binary);
        assert_eq!(id.to_string(), text);
        assert_eq!(id, operation.id(algorithm).unwrap());
    }
}

#[test]
fn exact_marker_key_and_reference_edges_are_exposed() {
    let decoded =
        AnyObject::decode(&operation().encode().unwrap(), CanonicalLimits::metadata()).unwrap();
    let transitions = decoded.bound_reference_transitions().unwrap().unwrap();
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].key, ReferenceKey::Marker([0x55; 16]));
    assert_eq!(
        transitions[0].before.as_ref().unwrap().kind,
        ObjectKind::Marker
    );
    assert_eq!(transitions[0].after.kind, ObjectKind::Marker);
    let edges = decoded.references().unwrap();
    assert!(
        edges
            .iter()
            .any(|edge| edge.role == ReferenceRole::OperationBefore)
    );
    assert!(
        edges
            .iter()
            .any(|edge| edge.role == ReferenceRole::OperationAfter)
    );
}

#[test]
fn schema_support_registry_is_kind_specific() {
    assert!(ObjectKind::Operation.supports_schema(0));
    assert!(ObjectKind::Operation.supports_schema(1));
    assert_eq!(ObjectKind::Operation.latest_schema_version(), 1);
    for kind in [ObjectKind::Chunk, ObjectKind::Marker, ObjectKind::LineState] {
        assert!(kind.supports_schema(0));
        assert!(!kind.supports_schema(1));
        assert_eq!(kind.latest_schema_version(), 0);
    }
}

#[test]
fn encoder_rejects_unbound_kinds_kind_mismatches_and_duplicate_keys() {
    let mut invalid = operation();
    let OperationActionV1::BoundTransition { key, .. } = &mut invalid.actions[0] else {
        unreachable!()
    };
    *key = ReferenceKey::OperationHead;
    assert!(invalid.encode().is_err());

    let mut invalid = operation();
    let OperationActionV1::BoundTransition { after, .. } = &mut invalid.actions[0] else {
        unreachable!()
    };
    after.kind = ObjectKind::Release;
    assert!(invalid.encode().is_err());

    let mut invalid = operation();
    invalid.actions.push(invalid.actions[0].clone());
    assert!(invalid.encode().is_err());
}

#[test]
fn decoder_rejects_every_bound_transition_shape_escape() {
    let bytes = operation().encode().unwrap();
    let original = decode_canonical(&bytes, CanonicalLimits::metadata()).unwrap();
    let mutate_action = |mut value: Value, mutation: fn(&mut Vec<(u64, Value)>)| {
        let actions = &mut map_mut(&mut value)
            .iter_mut()
            .find(|(k, _)| *k == 8)
            .unwrap()
            .1;
        let Value::Array(actions) = actions else {
            panic!("actions")
        };
        mutation(map_mut(&mut actions[0]));
        value.encode().unwrap()
    };

    let legacy = mutate_action(original.clone(), |action| {
        action.iter_mut().find(|(k, _)| *k == 0).unwrap().1 = Value::Unsigned(0);
    });
    assert!(AnyObject::decode(&legacy, CanonicalLimits::metadata()).is_err());
    let missing_key = mutate_action(original.clone(), |action| action.retain(|(k, _)| *k != 1));
    assert!(AnyObject::decode(&missing_key, CanonicalLimits::metadata()).is_err());
    let missing_after = mutate_action(original.clone(), |action| action.retain(|(k, _)| *k != 3));
    assert!(AnyObject::decode(&missing_after, CanonicalLimits::metadata()).is_err());
    let bad_key_kind = mutate_action(original.clone(), |action| {
        let key = &mut action.iter_mut().find(|(k, _)| *k == 1).unwrap().1;
        map_mut(key).iter_mut().find(|(k, _)| *k == 0).unwrap().1 = Value::Unsigned(99);
    });
    assert!(AnyObject::decode(&bad_key_kind, CanonicalLimits::metadata()).is_err());
    let short_key = mutate_action(original.clone(), |action| {
        let key = &mut action.iter_mut().find(|(k, _)| *k == 1).unwrap().1;
        map_mut(key).iter_mut().find(|(k, _)| *k == 1).unwrap().1 = Value::Bytes(vec![0; 15]);
    });
    assert!(AnyObject::decode(&short_key, CanonicalLimits::metadata()).is_err());
    let missing_stable_id = mutate_action(original.clone(), |action| {
        let key = &mut action.iter_mut().find(|(k, _)| *k == 1).unwrap().1;
        map_mut(key).retain(|(k, _)| *k != 1);
    });
    assert!(AnyObject::decode(&missing_stable_id, CanonicalLimits::metadata()).is_err());
    let operation_head_with_id = mutate_action(original.clone(), |action| {
        let key = &mut action.iter_mut().find(|(k, _)| *k == 1).unwrap().1;
        map_mut(key).iter_mut().find(|(k, _)| *k == 0).unwrap().1 = Value::Unsigned(3);
    });
    assert!(AnyObject::decode(&operation_head_with_id, CanonicalLimits::metadata()).is_err());
    let operation_head = mutate_action(original.clone(), |action| {
        let key = &mut action.iter_mut().find(|(k, _)| *k == 1).unwrap().1;
        let fields = map_mut(key);
        fields.retain(|(k, _)| *k == 0);
        fields[0].1 = Value::Unsigned(3);
    });
    assert!(AnyObject::decode(&operation_head, CanonicalLimits::metadata()).is_err());
    let line_key = mutate_action(original.clone(), |action| {
        let key = &mut action.iter_mut().find(|(k, _)| *k == 1).unwrap().1;
        map_mut(key).iter_mut().find(|(k, _)| *k == 0).unwrap().1 = Value::Unsigned(1);
    });
    assert!(AnyObject::decode(&line_key, CanonicalLimits::metadata()).is_err());
    let wrong_after_kind = mutate_action(original.clone(), |action| {
        let after = &mut action.iter_mut().find(|(k, _)| *k == 3).unwrap().1;
        map_mut(after).iter_mut().find(|(k, _)| *k == 0).unwrap().1 =
            Value::Unsigned(ObjectKind::Release as u64);
    });
    assert!(AnyObject::decode(&wrong_after_kind, CanonicalLimits::metadata()).is_err());
    let wrong_before_kind = mutate_action(original.clone(), |action| {
        let before = &mut action.iter_mut().find(|(k, _)| *k == 2).unwrap().1;
        map_mut(before).iter_mut().find(|(k, _)| *k == 0).unwrap().1 =
            Value::Unsigned(ObjectKind::Release as u64);
    });
    assert!(AnyObject::decode(&wrong_before_kind, CanonicalLimits::metadata()).is_err());
    let unknown_action_field = mutate_action(original.clone(), |action| {
        action.push((4, Value::Unsigned(0)));
    });
    assert!(AnyObject::decode(&unknown_action_field, CanonicalLimits::metadata()).is_err());

    let mut duplicate = original.clone();
    let actions = &mut map_mut(&mut duplicate)
        .iter_mut()
        .find(|(k, _)| *k == 8)
        .unwrap()
        .1;
    let Value::Array(actions) = actions else {
        panic!("actions")
    };
    actions.push(actions[0].clone());
    assert!(AnyObject::decode(&duplicate.encode().unwrap(), CanonicalLimits::metadata()).is_err());

    let mut newer = original.clone();
    map_mut(&mut newer)
        .iter_mut()
        .find(|(k, _)| *k == 1)
        .unwrap()
        .1 = Value::Unsigned(2);
    assert!(AnyObject::decode(&newer.encode().unwrap(), CanonicalLimits::metadata()).is_err());

    let mut non_operation: Value = decode_canonical(
        &hex::decode(
            serde_json::from_str::<Vec<serde_json::Value>>(include_str!("vectors/schema-v0.json"))
                .unwrap()[0]["canonical_cbor_hex"]
                .as_str()
                .unwrap(),
        )
        .unwrap(),
        CanonicalLimits::bulk(),
    )
    .unwrap();
    map_mut(&mut non_operation)
        .iter_mut()
        .find(|(k, _)| *k == 1)
        .unwrap()
        .1 = Value::Unsigned(1);
    assert!(AnyObject::decode(&non_operation.encode().unwrap(), CanonicalLimits::bulk()).is_err());

    assert_eq!(
        map(&original).iter().find(|(k, _)| *k == 1).unwrap().1,
        Value::Unsigned(1)
    );
}

#[test]
#[ignore = "explicit deterministic fixture regeneration"]
fn generate_operation_v1_vector() {
    let operation = operation();
    let encoded = operation.encode().unwrap();
    let unsigned = operation.unsigned_encode().unwrap();
    let decoded = AnyObject::decode(&encoded, CanonicalLimits::metadata()).unwrap();
    let sha = decoded.id(HashAlgorithm::Sha256).unwrap();
    let blake = decoded.id(HashAlgorithm::Blake3_256).unwrap();
    let fixture = Fixture {
        schema: 1,
        canonical_cbor_hex: hex::encode(&encoded),
        unsigned_cbor_hex: hex::encode(&unsigned),
        signing_preimage_hex: hex::encode(
            operation.signing_preimage(&operation.signature).unwrap(),
        ),
        sha256_binary_id_hex: hex::encode(sha.to_bytes()),
        sha256_text_id: sha.to_string(),
        blake3_binary_id_hex: hex::encode(blake.to_bytes()),
        blake3_text_id: blake.to_string(),
    };
    std::fs::write(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/vectors/operation-v1.json"
        ),
        format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap()),
    )
    .unwrap();
}
