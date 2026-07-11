use rgit_objects::{
    ActorId, AnyObject, Blob, BlobContent, CanonicalLimits, CanonicalObject, Capability, ChangeId,
    ChangeRevision, ChangeState, Chunk, Conflict, ConflictRegion, DeviceId, FileMode, Grant,
    HashAlgorithm, LineAdvanceDeclaration, LineId, LineState, Manifest, ManifestEntry,
    ManifestTarget, Marker, MarkerKind, ObjectId, ObjectKind, Operation, OperationAction,
    PathSegment, Policy, PolicyId, PolicyRef, PortablePath, Principal, PrincipalKind,
    RedactionMode, Release, SecretRef, Signature, SignatureAlgorithm, SignaturePurpose, Snapshot,
    Subproject, TypedObjectRef, WallTime,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Deserialize, Serialize)]
struct Fixture {
    name: String,
    kind: u64,
    canonical_cbor_hex: String,
    preimage_hex: String,
    sha256_binary_id_hex: String,
    sha256_text_id: String,
    blake3_binary_id_hex: String,
    blake3_text_id: String,
}

fn fixture(name: &str, kind: ObjectKind, encoded: &[u8]) -> Fixture {
    let preimage = ObjectId::preimage(kind as u64, 0, encoded);
    let object = AnyObject::decode(encoded, CanonicalLimits::default()).unwrap();
    let sha = object.id(HashAlgorithm::Sha256).unwrap();
    let blake = object.id(HashAlgorithm::Blake3_256).unwrap();
    Fixture {
        name: name.to_owned(),
        kind: kind as u64,
        canonical_cbor_hex: hex::encode(encoded),
        preimage_hex: hex::encode(preimage),
        sha256_binary_id_hex: hex::encode(sha.to_bytes()),
        sha256_text_id: sha.to_string(),
        blake3_binary_id_hex: hex::encode(blake.to_bytes()),
        blake3_text_id: blake.to_string(),
    }
}

fn zero_id() -> ObjectId {
    let mut bytes = vec![0, 0x12, 32];
    bytes.extend([0; 32]);
    ObjectId::from_bytes(&bytes).unwrap()
}

fn policy_ref() -> PolicyRef {
    PolicyRef {
        policy_id: PolicyId::from_bytes([0; 16]),
        version: zero_id(),
    }
}

fn signature(purpose: SignaturePurpose) -> Signature {
    Signature::new(
        SignatureAlgorithm::Ed25519,
        ActorId::from_bytes([0x44; 16]),
        [0x6b; 32],
        hex::decode("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b")
            .unwrap()
            .try_into()
            .unwrap(),
        purpose,
    )
    .unwrap()
}

fn fixture_objects() -> Vec<(&'static str, ObjectKind, Vec<u8>)> {
    let oid = zero_id();
    let policy = policy_ref();
    let wall_time = WallTime {
        utc_seconds: -1,
        offset_seconds: 3_600,
    };
    let operation = Operation {
        policy_ref: policy.clone(),
        parents: vec![],
        actor: ActorId::from_bytes([0x22; 16]),
        device: DeviceId::from_bytes([0x33; 16]),
        logical_time: u64::from(u16::MAX) + 1,
        wall_time: wall_time.clone(),
        actions: vec![OperationAction::LineAdvance(Box::new(
            LineAdvanceDeclaration {
                policy_ref: policy.clone(),
                line_id: LineId::from_bytes([0x55; 16]),
                display_name: "main".to_owned(),
                head_snapshot: oid.clone(),
                generation: 0,
                previous_state: None,
                integration_policy: policy.clone(),
                approval_policy: policy.clone(),
                release_policy: policy.clone(),
                visibility_policy: policy.clone(),
            },
        ))],
        inverse_payloads: vec![],
        public_envelope: None,
        private_payload: None,
        signature: signature(SignaturePurpose::Operation),
        client_implementation: "rgit/0".to_owned(),
    };
    let operation_id = operation.id(HashAlgorithm::Sha256).unwrap();
    let line_state = LineState {
        policy_ref: policy.clone(),
        line_id: LineId::from_bytes([0x55; 16]),
        display_name: "main".to_owned(),
        head_snapshot: oid.clone(),
        generation: 0,
        previous_state: None,
        integration_policy: policy.clone(),
        approval_policy: policy.clone(),
        release_policy: policy.clone(),
        visibility_policy: policy.clone(),
        transaction_operation: operation_id,
        signature: signature(SignaturePurpose::LineState),
    };
    line_state.validate_transaction(&operation).unwrap();
    let mut mismatched_operation = operation.clone();
    let OperationAction::LineAdvance(declaration) = &mut mismatched_operation.actions[0] else {
        unreachable!()
    };
    declaration.display_name = "other".to_owned();
    let mut state_for_mismatch = line_state.clone();
    state_for_mismatch.transaction_operation =
        mismatched_operation.id(HashAlgorithm::Sha256).unwrap();
    assert!(
        state_for_mismatch
            .validate_transaction(&mismatched_operation)
            .is_err()
    );
    let mut duplicate_declaration = operation.clone();
    duplicate_declaration
        .actions
        .push(operation.actions[0].clone());
    let mut state_for_duplicate = line_state.clone();
    state_for_duplicate.transaction_operation =
        duplicate_declaration.id(HashAlgorithm::Sha256).unwrap();
    assert!(
        state_for_duplicate
            .validate_transaction(&duplicate_declaration)
            .is_err()
    );
    let mut wrong_operation_id = line_state.clone();
    wrong_operation_id.transaction_operation = zero_id();
    assert!(wrong_operation_id.validate_transaction(&operation).is_err());
    let mut objects = Vec::new();
    macro_rules! add {
        ($name:literal, $object:expr) => {{
            let object = $object;
            let encoded = object.encode().unwrap();
            let kind = AnyObject::decode(&encoded, CanonicalLimits::default())
                .unwrap()
                .decoded()
                .kind();
            objects.push(($name, kind, encoded));
        }};
    }

    add!(
        "chunk",
        Chunk {
            policy_ref: policy.clone(),
            bytes: b"abc".to_vec(),
        }
    );
    add!(
        "blob",
        Blob {
            policy_ref: policy.clone(),
            byte_length: 2,
            content: BlobContent::Inline(b"hi".to_vec()),
            chunk_profile: None,
            content_hint: Some("text/plain".to_owned()),
        }
    );
    add!(
        "secret-ref",
        SecretRef {
            policy_ref: policy.clone(),
            provider_kind: "vault".to_owned(),
            locator: "kv/app".to_owned(),
            exact_version: Some("7".to_owned()),
            value_schema_id: vec![1, 2],
            materialization_target: "API_KEY".to_owned(),
            required_capability: "materialize".to_owned(),
            encrypted_development_value: Some(oid.clone()),
        }
    );
    add!(
        "manifest",
        Manifest {
            policy_ref: policy.clone(),
            entries: vec![ManifestEntry {
                name: PathSegment::new_portable("README.md").unwrap(),
                target: ManifestTarget::File {
                    blob: oid.clone(),
                    mode: FileMode::Executable,
                },
                policy_ref: policy.clone(),
            }],
        }
    );
    add!(
        "subproject",
        Subproject {
            policy_ref: policy.clone(),
            system_kind: "git".to_owned(),
            repository_identity: b"repo".to_vec(),
            revision: b"rev".to_vec(),
            native_projection: Some(oid.clone()),
        }
    );
    add!(
        "snapshot",
        Snapshot {
            policy_ref: policy.clone(),
            root_manifest: oid.clone(),
            parents: vec![],
            change_id: ChangeId::from_bytes([0x11; 16]),
            author: ActorId::from_bytes([0x22; 16]),
            device: DeviceId::from_bytes([0x33; 16]),
            logical_time: 24,
            wall_time: wall_time.clone(),
            message_blob: Some(oid.clone()),
        }
    );
    add!(
        "change-revision",
        ChangeRevision {
            policy_ref: policy.clone(),
            change_id: ChangeId::from_bytes([0x11; 16]),
            previous_revision: None,
            title_blob: oid.clone(),
            description_blob: oid.clone(),
            base_snapshot: oid.clone(),
            current_snapshot: oid.clone(),
            target_line: LineId::from_bytes([0x55; 16]),
            observed_generation: 256,
            owner: ActorId::from_bytes([0x22; 16]),
            author: ActorId::from_bytes([0x44; 16]),
            state: ChangeState::Open,
            review_policy: policy.clone(),
            landing_policy: policy.clone(),
        }
    );
    add!("line-state", line_state);
    add!(
        "conflict",
        Conflict {
            policy_ref: policy.clone(),
            base: TypedObjectRef {
                kind: ObjectKind::Blob,
                id: oid.clone(),
            },
            left: TypedObjectRef {
                kind: ObjectKind::Blob,
                id: oid.clone(),
            },
            right: TypedObjectRef {
                kind: ObjectKind::Blob,
                id: oid.clone(),
            },
            path: PortablePath::new(vec![
                PathSegment::new_portable("src").unwrap(),
                PathSegment::new_portable("lib.rs").unwrap(),
            ])
            .unwrap(),
            conflict_kind: rgit_objects::ConflictKind::AddAdd,
            merge_driver: "text".to_owned(),
            merge_driver_version: "1".to_owned(),
            regions: vec![ConflictRegion { start: 0, end: 2 }],
        }
    );
    add!("operation", operation);
    add!(
        "marker",
        Marker {
            policy_ref: policy.clone(),
            marker_kind: MarkerKind::Review,
            target: TypedObjectRef {
                kind: ObjectKind::Snapshot,
                id: oid.clone(),
            },
            issuer: ActorId::from_bytes([0x44; 16]),
            issue_time: wall_time.clone(),
            typed_payload: vec![0xa5],
            signature: signature(SignaturePurpose::Marker),
        }
    );
    add!(
        "release",
        Release {
            policy_ref: policy.clone(),
            source_line: LineId::from_bytes([0x55; 16]),
            source_generation: u64::from(u32::MAX) + 1,
            source_snapshot: oid.clone(),
            audience_policy: policy.clone(),
            projection_rules: oid.clone(),
            projected_root: oid.clone(),
            projection_proof: oid.clone(),
            version_identifier: "v1.0.0".to_owned(),
            release_notes_blob: None,
            build_provenance: vec![],
            artifacts: vec![],
            policy_decision_evidence: vec![],
            issue_time: wall_time.clone(),
            signatures: vec![signature(SignaturePurpose::Release)],
        }
    );
    add!(
        "policy",
        Policy {
            policy_ref: None,
            policy_id: PolicyId::from_bytes([0x66; 16]),
            version_sequence: 0,
            previous_version: None,
            principals: vec![Principal {
                kind: PrincipalKind::Actor,
                identifier: vec![0x44],
            }],
            grants: vec![Grant {
                principal_index: 0,
                capabilities: vec![Capability::Discover, Capability::Read],
            }],
            redaction_mode: RedactionMode::Omit,
            derivation_rule: rgit_objects::DerivationRule::NoDerivation,
            declassification_requirements: vec![],
            key_epoch: u64::MAX,
            key_envelope_set: oid,
            administrators: vec![ActorId::from_bytes([0x44; 16])],
            activation_constraints: vec![],
            signatures: vec![signature(SignaturePurpose::Policy)],
        }
    );
    objects
}

#[test]
fn every_schema_kind_matches_committed_cross_platform_vectors() {
    let fixtures: Vec<Fixture> =
        serde_json::from_str(include_str!("vectors/schema-v0.json")).unwrap();
    let objects = fixture_objects();
    assert_eq!(fixtures.len(), 13);
    assert_eq!(objects.len(), 13);

    for ((name, kind, encoded), fixture) in objects.into_iter().zip(fixtures) {
        assert_eq!(fixture.name, name);
        assert_eq!(fixture.kind, kind as u64);
        assert_eq!(hex::encode(&encoded), fixture.canonical_cbor_hex);

        let preimage = ObjectId::preimage(kind as u64, 0, &encoded);
        assert_eq!(hex::encode(&preimage), fixture.preimage_hex);

        let sha =
            ObjectId::from_bytes(&hex::decode(&fixture.sha256_binary_id_hex).unwrap()).unwrap();
        assert_eq!(sha.algorithm(), HashAlgorithm::Sha256);
        assert_eq!(sha.to_string(), fixture.sha256_text_id);
        assert_eq!(
            sha.digest().as_slice(),
            Sha256::digest(&preimage).as_slice()
        );

        let blake =
            ObjectId::from_bytes(&hex::decode(&fixture.blake3_binary_id_hex).unwrap()).unwrap();
        assert_eq!(blake.algorithm(), HashAlgorithm::Blake3_256);
        assert_eq!(blake.to_string(), fixture.blake3_text_id);
        assert_eq!(blake.digest(), blake3::hash(&preimage).as_bytes());

        let decoded = AnyObject::decode(&encoded, CanonicalLimits::default()).unwrap();
        assert_eq!(decoded.decoded().kind(), kind);
        assert_eq!(decoded.id(HashAlgorithm::Sha256).unwrap(), sha);
        assert_eq!(decoded.id(HashAlgorithm::Blake3_256).unwrap(), blake);
    }
}

#[test]
#[ignore = "explicit deterministic fixture regeneration"]
fn generate_schema_vectors() {
    let generated: Vec<_> = fixture_objects()
        .into_iter()
        .map(|(name, kind, encoded)| fixture(name, kind, &encoded))
        .collect();
    std::fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/schema-v0.json"),
        format!("{}\n", serde_json::to_string_pretty(&generated).unwrap()),
    )
    .unwrap();
}

#[test]
fn frozen_object_kind_assignments_are_exhaustively_pinned() {
    let expected = [
        ObjectKind::Chunk,
        ObjectKind::Blob,
        ObjectKind::SecretRef,
        ObjectKind::Manifest,
        ObjectKind::Subproject,
        ObjectKind::Snapshot,
        ObjectKind::ChangeRevision,
        ObjectKind::LineState,
        ObjectKind::Conflict,
        ObjectKind::Operation,
        ObjectKind::Marker,
        ObjectKind::Release,
        ObjectKind::Policy,
        ObjectKind::RepositoryRoot,
        ObjectKind::Identity,
        ObjectKind::GroupMembership,
        ObjectKind::KeyEnvelopeSet,
        ObjectKind::ChangeRelation,
        ObjectKind::ConflictResolution,
        ObjectKind::ReviewEvidence,
        ObjectKind::ApprovalEvidence,
        ObjectKind::CiEvidence,
        ObjectKind::PolicyDecisionEvidence,
        ObjectKind::ProjectionRules,
        ObjectKind::ProjectionProof,
        ObjectKind::BuildProvenance,
        ObjectKind::Artifact,
        ObjectKind::OperationPayload,
        ObjectKind::View,
        ObjectKind::Migration,
        ObjectKind::Ruleset,
    ];
    for (number, kind) in (1_u64..=31).zip(expected) {
        assert_eq!(kind as u64, number);
        assert_eq!(ObjectKind::try_from(number), Ok(kind));
    }
    assert_eq!(ObjectKind::try_from(0), Err(0));
    assert_eq!(ObjectKind::try_from(32), Err(32));
}
