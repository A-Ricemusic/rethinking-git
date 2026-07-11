use rgit_objects::{
    ActorId, AnyObject, CanonicalLimits, HashAlgorithm, ObjectId, ObjectKind, ReferenceRole,
    SignatureAlgorithm, SignaturePurpose, Value, decode_canonical, signing_preimage,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
struct SchemaFixture {
    name: String,
    kind: u64,
    canonical_cbor_hex: String,
    sha256_binary_id_hex: String,
    blake3_binary_id_hex: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SignedFixture {
    name: String,
    kind: u64,
    unsigned_cbor_hex: String,
    signing_preimage_hex: String,
    canonical_cbor_hex: String,
    sha256_binary_id_hex: String,
    blake3_binary_id_hex: String,
}

fn signature_field(kind: ObjectKind) -> u64 {
    match kind {
        ObjectKind::LineState => 13,
        ObjectKind::Operation => 12,
        ObjectKind::Marker => 8,
        ObjectKind::Release => 16,
        ObjectKind::Policy => 15,
        _ => panic!("not a signed schema-0 kind"),
    }
}

fn purpose(kind: ObjectKind) -> SignaturePurpose {
    match kind {
        ObjectKind::LineState => SignaturePurpose::LineState,
        ObjectKind::Operation => SignaturePurpose::Operation,
        ObjectKind::Marker => SignaturePurpose::Marker,
        ObjectKind::Release => SignaturePurpose::Release,
        ObjectKind::Policy => SignaturePurpose::Policy,
        _ => panic!("not a signed schema-0 kind"),
    }
}

fn object_map(value: &Value) -> &[(u64, Value)] {
    let Value::Map(map) = value else {
        panic!("fixture object is not a map")
    };
    map
}

fn object_map_mut(value: &mut Value) -> &mut Vec<(u64, Value)> {
    let Value::Map(map) = value else {
        panic!("fixture object is not a map")
    };
    map
}

fn unsigned_projection(value: &Value, kind: ObjectKind) -> Value {
    let mut fields = object_map(value).to_vec();
    let field = signature_field(kind);
    let before = fields.len();
    fields.retain(|(key, _)| *key != field);
    assert_eq!(fields.len() + 1, before);
    Value::Map(fields)
}

fn first_signature_mut(value: &mut Value, kind: ObjectKind) -> &mut Vec<(u64, Value)> {
    let field = signature_field(kind);
    let signature_value = &mut object_map_mut(value)
        .iter_mut()
        .find(|(key, _)| *key == field)
        .unwrap()
        .1;
    if matches!(kind, ObjectKind::Release | ObjectKind::Policy) {
        let Value::Array(signatures) = signature_value else {
            panic!("not an array")
        };
        let Value::Map(signature) = &mut signatures[0] else {
            panic!("not a signature")
        };
        signature
    } else {
        let Value::Map(signature) = signature_value else {
            panic!("not a signature")
        };
        signature
    }
}

fn fixtures() -> Vec<(SignedFixture, Value, ObjectKind)> {
    let vectors: Vec<SignedFixture> =
        serde_json::from_str(include_str!("vectors/signed-v0.json")).unwrap();
    vectors
        .into_iter()
        .map(|fixture| {
            let kind = ObjectKind::try_from(fixture.kind).unwrap();
            let bytes = hex::decode(&fixture.canonical_cbor_hex).unwrap();
            let value = decode_canonical(&bytes, CanonicalLimits::default()).unwrap();
            (fixture, value, kind)
        })
        .collect()
}

#[test]
fn signed_schema_vectors_pin_projection_preimage_final_cbor_and_ids() {
    let fixtures = fixtures();
    assert_eq!(fixtures.len(), 5);
    for (fixture, value, kind) in fixtures {
        let unsigned = unsigned_projection(&value, kind).encode().unwrap();
        assert_eq!(hex::encode(&unsigned), fixture.unsigned_cbor_hex);
        let preimage = signing_preimage(
            SignatureAlgorithm::Ed25519,
            purpose(kind),
            ActorId::from_bytes([0x44; 16]),
            &[0x6b; 32],
            kind,
            0,
            &unsigned,
        );
        assert_eq!(hex::encode(preimage), fixture.signing_preimage_hex);

        let bytes = hex::decode(&fixture.canonical_cbor_hex).unwrap();
        let decoded = AnyObject::decode(&bytes, CanonicalLimits::default()).unwrap();
        for (algorithm, expected) in [
            (HashAlgorithm::Sha256, &fixture.sha256_binary_id_hex),
            (HashAlgorithm::Blake3_256, &fixture.blake3_binary_id_hex),
        ] {
            assert_eq!(
                decoded.id(algorithm).unwrap(),
                ObjectId::from_bytes(&hex::decode(expected).unwrap()).unwrap()
            );
        }
    }
}

#[test]
fn every_bound_component_has_a_mutation_negative() {
    for (fixture, value, kind) in fixtures() {
        let original_unsigned = unsigned_projection(&value, kind).encode().unwrap();
        let original_preimage = signing_preimage(
            SignatureAlgorithm::Ed25519,
            purpose(kind),
            ActorId::from_bytes([0x44; 16]),
            &[0x6b; 32],
            kind,
            0,
            &original_unsigned,
        );

        let mut content_mutation = value.clone();
        let content_key = match kind {
            ObjectKind::LineState => 4,
            ObjectKind::Operation => 13,
            ObjectKind::Marker => 7,
            ObjectKind::Release => 10,
            ObjectKind::Policy => 14,
            _ => unreachable!(),
        };
        let content = &mut object_map_mut(&mut content_mutation)
            .iter_mut()
            .find(|(key, _)| *key == content_key)
            .unwrap()
            .1;
        match content {
            Value::Text(text) => text.push('x'),
            Value::Bytes(bytes) => bytes.push(0xff),
            _ => unreachable!(),
        }
        let mutated_bytes = content_mutation.encode().unwrap();
        AnyObject::decode(&mutated_bytes, CanonicalLimits::default()).unwrap();
        let mutated_unsigned = unsigned_projection(&content_mutation, kind)
            .encode()
            .unwrap();
        assert_ne!(mutated_unsigned, original_unsigned, "{}", fixture.name);
        let preimage_mutations = [
            signing_preimage(
                SignatureAlgorithm::Ed25519,
                purpose(kind),
                ActorId::from_bytes([0x44; 16]),
                &[0x6b; 32],
                kind,
                0,
                &mutated_unsigned,
            ),
            signing_preimage(
                SignatureAlgorithm::Ed25519,
                purpose(kind),
                ActorId::from_bytes([0x45; 16]),
                &[0x6b; 32],
                kind,
                0,
                &original_unsigned,
            ),
            signing_preimage(
                SignatureAlgorithm::Ed25519,
                purpose(kind),
                ActorId::from_bytes([0x44; 16]),
                &[0x6c; 32],
                kind,
                0,
                &original_unsigned,
            ),
            signing_preimage(
                SignatureAlgorithm::Ed25519,
                SignaturePurpose::try_from((purpose(kind) as u64 + 1) % 5).unwrap(),
                ActorId::from_bytes([0x44; 16]),
                &[0x6b; 32],
                kind,
                0,
                &original_unsigned,
            ),
            signing_preimage(
                SignatureAlgorithm::Ed25519,
                purpose(kind),
                ActorId::from_bytes([0x44; 16]),
                &[0x6b; 32],
                ObjectKind::Chunk,
                0,
                &original_unsigned,
            ),
            signing_preimage(
                SignatureAlgorithm::Ed25519,
                purpose(kind),
                ActorId::from_bytes([0x44; 16]),
                &[0x6b; 32],
                kind,
                1,
                &original_unsigned,
            ),
        ];
        for mutation in preimage_mutations {
            assert_ne!(mutation, original_preimage, "{}", fixture.name);
        }

        for (signature_key, replacement) in [
            (0, Value::Unsigned(99)),
            (2, Value::Bytes(vec![0x6b; 31])),
            (3, Value::Bytes(vec![0x73; 63])),
            (4, Value::Unsigned((purpose(kind) as u64 + 1) % 5)),
        ] {
            let mut invalid = value.clone();
            first_signature_mut(&mut invalid, kind)
                .iter_mut()
                .find(|(key, _)| *key == signature_key)
                .unwrap()
                .1 = replacement;
            assert!(
                AnyObject::decode(&invalid.encode().unwrap(), CanonicalLimits::default()).is_err(),
                "{} accepted invalid signature field {signature_key}",
                fixture.name,
            );
        }

        let mut changed_signature_bytes = value.clone();
        let signature = first_signature_mut(&mut changed_signature_bytes, kind);
        let Value::Bytes(bytes) = &mut signature.iter_mut().find(|(key, _)| *key == 3).unwrap().1
        else {
            unreachable!()
        };
        bytes[0] ^= 1;
        assert_eq!(
            unsigned_projection(&changed_signature_bytes, kind)
                .encode()
                .unwrap(),
            original_unsigned,
        );
        assert_ne!(
            changed_signature_bytes.encode().unwrap(),
            hex::decode(&fixture.canonical_cbor_hex).unwrap(),
        );
        let changed = AnyObject::decode(
            &changed_signature_bytes.encode().unwrap(),
            CanonicalLimits::default(),
        )
        .unwrap();
        assert_ne!(
            hex::encode(changed.id(HashAlgorithm::Sha256).unwrap().to_bytes()),
            fixture.sha256_binary_id_hex,
        );
        assert_ne!(
            hex::encode(changed.id(HashAlgorithm::Blake3_256).unwrap().to_bytes()),
            fixture.blake3_binary_id_hex,
        );
    }
}

#[test]
fn line_advance_encoding_is_acyclic_and_closed() {
    let fixtures = fixtures();
    let (_, line_state, _) = fixtures
        .iter()
        .find(|(_, _, kind)| *kind == ObjectKind::LineState)
        .unwrap();
    let (_, operation, _) = fixtures
        .iter()
        .find(|(_, _, kind)| *kind == ObjectKind::Operation)
        .unwrap();
    let operation_id = AnyObject::decode(&operation.encode().unwrap(), CanonicalLimits::default())
        .unwrap()
        .id(HashAlgorithm::Sha256)
        .unwrap();
    let line_map = object_map(line_state);
    assert_eq!(
        line_map.iter().find(|(key, _)| *key == 12).unwrap().1,
        Value::Bytes(operation_id.to_bytes()),
    );

    let actions = &object_map(operation)
        .iter()
        .find(|(key, _)| *key == 8)
        .unwrap()
        .1;
    let Value::Array(actions) = actions else {
        unreachable!()
    };
    let Value::Map(action) = &actions[0] else {
        unreachable!()
    };
    let Value::Map(declaration) = &action.iter().find(|(key, _)| *key == 3).unwrap().1 else {
        unreachable!()
    };
    for (declaration_key, line_key) in [
        (0, 2),
        (1, 3),
        (2, 4),
        (3, 5),
        (4, 6),
        (6, 8),
        (7, 9),
        (8, 10),
        (9, 11),
    ] {
        assert_eq!(
            declaration
                .iter()
                .find(|(key, _)| *key == declaration_key)
                .unwrap()
                .1,
            line_map.iter().find(|(key, _)| *key == line_key).unwrap().1,
        );
    }
    assert!(declaration.iter().all(|(key, _)| *key != 5));
    assert!(declaration.iter().all(|(key, _)| *key != 10));

    let mut operation = operation.clone();
    let decoded =
        AnyObject::decode(&operation.encode().unwrap(), CanonicalLimits::default()).unwrap();
    let edges = decoded.references().unwrap();
    assert!(
        edges
            .iter()
            .any(|edge| edge.role == ReferenceRole::OperationLineHeadSnapshot)
    );
    assert!(
        !edges
            .iter()
            .any(|edge| edge.role == ReferenceRole::OperationAfter)
    );

    let actions = &mut object_map_mut(&mut operation)
        .iter_mut()
        .find(|(key, _)| *key == 8)
        .unwrap()
        .1;
    let Value::Array(actions) = actions else {
        unreachable!()
    };
    let Value::Map(action) = &mut actions[0] else {
        unreachable!()
    };
    let Value::Map(declaration) = &mut action.iter_mut().find(|(key, _)| *key == 3).unwrap().1
    else {
        unreachable!()
    };
    declaration.push((10, Value::Bytes(vec![0; 35])));
    assert!(AnyObject::decode(&operation.encode().unwrap(), CanonicalLimits::default()).is_err());

    let mut generic_after = operation;
    let actions = &mut object_map_mut(&mut generic_after)
        .iter_mut()
        .find(|(key, _)| *key == 8)
        .unwrap()
        .1;
    *actions = Value::Array(vec![Value::Map(vec![
        (0, Value::Unsigned(0)),
        (
            2,
            Value::Map(vec![
                (0, Value::Unsigned(ObjectKind::LineState as u64)),
                (
                    1,
                    Value::Bytes(vec![0, 0x12, 32].into_iter().chain([0; 32]).collect()),
                ),
            ]),
        ),
    ])]);
    assert!(
        AnyObject::decode(&generic_after.encode().unwrap(), CanonicalLimits::default()).is_err()
    );
}

#[test]
fn signatures_reject_absence_empty_sets_duplicates_and_noncanonical_order() {
    for (fixture, value, kind) in fixtures() {
        let field = signature_field(kind);

        let mut absent = value.clone();
        object_map_mut(&mut absent).retain(|(key, _)| *key != field);
        assert!(
            AnyObject::decode(&absent.encode().unwrap(), CanonicalLimits::default()).is_err(),
            "{} accepted an absent signature field",
            fixture.name,
        );

        let mut empty_or_wrong_shape = value.clone();
        object_map_mut(&mut empty_or_wrong_shape)
            .iter_mut()
            .find(|(key, _)| *key == field)
            .unwrap()
            .1 = Value::Array(vec![]);
        assert!(
            AnyObject::decode(
                &empty_or_wrong_shape.encode().unwrap(),
                CanonicalLimits::default(),
            )
            .is_err(),
            "{} accepted an empty signature representation",
            fixture.name,
        );

        if !matches!(kind, ObjectKind::Release | ObjectKind::Policy) {
            continue;
        }

        let mut duplicate = value.clone();
        let Value::Array(signatures) = &mut object_map_mut(&mut duplicate)
            .iter_mut()
            .find(|(key, _)| *key == field)
            .unwrap()
            .1
        else {
            unreachable!()
        };
        signatures.push(signatures[0].clone());
        assert!(
            AnyObject::decode(&duplicate.encode().unwrap(), CanonicalLimits::default()).is_err(),
            "{} accepted duplicate signatures",
            fixture.name,
        );

        let mut unsorted = value;
        let Value::Array(signatures) = &mut object_map_mut(&mut unsorted)
            .iter_mut()
            .find(|(key, _)| *key == field)
            .unwrap()
            .1
        else {
            unreachable!()
        };
        let mut second_signature = signatures[0].clone();
        let Value::Map(second_fields) = &mut second_signature else {
            unreachable!()
        };
        second_fields
            .iter_mut()
            .find(|(key, _)| *key == 1)
            .unwrap()
            .1 = Value::Bytes(vec![0x45; 16]);
        signatures.push(second_signature);
        signatures.sort_by_key(|signature| signature.encode().unwrap());
        signatures.reverse();
        assert!(
            AnyObject::decode(&unsorted.encode().unwrap(), CanonicalLimits::default()).is_err(),
            "{} accepted noncanonical signature order",
            fixture.name,
        );
    }
}

#[test]
#[ignore = "explicit deterministic fixture regeneration"]
fn generate_signed_vectors() {
    let schemas: Vec<SchemaFixture> =
        serde_json::from_str(include_str!("vectors/schema-v0.json")).unwrap();
    let generated: Vec<_> = schemas
        .into_iter()
        .filter_map(|schema| {
            let kind = ObjectKind::try_from(schema.kind).ok()?;
            if !matches!(
                kind,
                ObjectKind::LineState
                    | ObjectKind::Operation
                    | ObjectKind::Marker
                    | ObjectKind::Release
                    | ObjectKind::Policy
            ) {
                return None;
            }
            let bytes = hex::decode(&schema.canonical_cbor_hex).unwrap();
            let value = decode_canonical(&bytes, CanonicalLimits::default()).unwrap();
            let unsigned = unsigned_projection(&value, kind).encode().unwrap();
            let preimage = signing_preimage(
                SignatureAlgorithm::Ed25519,
                purpose(kind),
                ActorId::from_bytes([0x44; 16]),
                &[0x6b; 32],
                kind,
                0,
                &unsigned,
            );
            Some(SignedFixture {
                name: schema.name,
                kind: schema.kind,
                unsigned_cbor_hex: hex::encode(unsigned),
                signing_preimage_hex: hex::encode(preimage),
                canonical_cbor_hex: schema.canonical_cbor_hex,
                sha256_binary_id_hex: schema.sha256_binary_id_hex,
                blake3_binary_id_hex: schema.blake3_binary_id_hex,
            })
        })
        .collect();
    std::fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/signed-v0.json"),
        format!("{}\n", serde_json::to_string_pretty(&generated).unwrap()),
    )
    .unwrap();
}
