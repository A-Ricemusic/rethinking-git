use rgit_objects::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Vector {
    name: String,
    kind: u64,
    canonical_cbor_hex: String,
    sha256_id: String,
    blake3_id: String,
    unsigned_cbor_hex: Option<String>,
    signing_preimage_hex: Option<String>,
    edges: Vec<String>,
    semantic_json: String,
}
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct BootstrapFixture {
    root_id: String,
    root_cbor_hex: String,
    root_policy_id: String,
    identity_ids: Vec<String>,
    key_envelope_set_id: String,
    genesis_operation_id: String,
    initial_line_state_ids: Vec<String>,
    dependency_order: Vec<String>,
}

fn id(byte: u8) -> ObjectId {
    let mut encoded = vec![0, 0x12, 32];
    encoded.extend([byte; 32]);
    ObjectId::from_bytes(&encoded).unwrap()
}
fn policy() -> PolicyRef {
    PolicyRef {
        policy_id: PolicyId::from_bytes([1; 16]),
        version: id(1),
    }
}
fn time() -> WallTime {
    WallTime {
        utc_seconds: 1,
        offset_seconds: 0,
    }
}
fn signature(purpose: SignaturePurpose) -> Signature {
    Signature::new(
        SignatureAlgorithm::Ed25519,
        ActorId::from_bytes([2; 16]),
        [3; 32],
        [purpose as u8 + 1; 64],
        purpose,
    )
    .unwrap()
}
fn typed(kind: ObjectKind, byte: u8) -> TypedObjectRef {
    TypedObjectRef { kind, id: id(byte) }
}
fn principal() -> Principal {
    Principal {
        kind: PrincipalKind::Actor,
        identifier: vec![2; 16],
    }
}

fn vector<T: CanonicalObject>(name: &str, object: &T, signed: Option<(&[u8], &[u8])>) -> Vector {
    let bytes = object.encode().unwrap();
    let decoded = AnyObject::decode(&bytes, CanonicalLimits::metadata()).unwrap();
    let edges = decoded
        .references()
        .unwrap()
        .into_iter()
        .map(|e| format!("{:?}:{:?}:{}", e.role, e.expected_kind, e.id))
        .collect();
    Vector {
        name: name.into(),
        kind: T::KIND as u64,
        canonical_cbor_hex: hex::encode(&bytes),
        sha256_id: object.id(HashAlgorithm::Sha256).unwrap().to_string(),
        blake3_id: object.id(HashAlgorithm::Blake3_256).unwrap().to_string(),
        unsigned_cbor_hex: signed.map(|v| hex::encode(v.0)),
        signing_preimage_hex: signed.map(|v| hex::encode(v.1)),
        edges,
        semantic_json: object.debug_json().unwrap(),
    }
}

fn vectors() -> Vec<Vector> {
    let p = policy();
    let actor = ActorId::from_bytes([2; 16]);
    let device = DeviceId::from_bytes([4; 16]);
    let key = PublicKeyRecord {
        algorithm: PublicKeyAlgorithm::Ed25519,
        key_id: [3; 32],
        public_key: vec![4; 32],
        not_before: None,
        not_after: None,
    };
    let identity = Identity::try_new(Identity {
        policy_ref: None,
        subject_kind: IdentitySubjectKind::Actor,
        subject: [2; 16],
        version: 0,
        previous: None,
        signing_keys: vec![key],
        encryption_keys: vec![],
        issuer: actor,
        status: IdentityStatus::Active,
        activation_operation: None,
        not_after_operation: None,
        signatures: vec![signature(SignaturePurpose::Identity)],
    })
    .unwrap();
    let root = RepositoryRoot::try_new(RepositoryRoot {
        repository_id: RepositoryId::from_bytes([9; 16]),
        root_policy: id(1),
        trusted_identities: vec![id(2)],
        bootstrap_key_envelope_set: id(3),
        genesis_operation: id(4),
        initial_line_states: vec![id(5)],
        filesystem_profile: FilesystemProfile::Portable,
        signatures: vec![signature(SignaturePurpose::RepositoryRoot)],
    })
    .unwrap();
    let membership = GroupMembership::try_new(GroupMembership {
        policy_ref: p.clone(),
        membership_id: MembershipId::from_bytes([1; 16]),
        group_id: GroupId::from_bytes([2; 16]),
        version: 0,
        previous: None,
        principal: principal(),
        state: MembershipState::Active,
        issuer: actor,
        activation_operation: id(4),
        not_after_operation: None,
        signatures: vec![signature(SignaturePurpose::GroupMembership)],
    })
    .unwrap();
    let envelopes = KeyEnvelopeSet::try_new(KeyEnvelopeSet {
        policy_ref: None,
        epoch: 0,
        suite: KeyEnvelopeSuite::X25519HkdfSha256Aes256Gcm,
        recipients: vec![RecipientEnvelope {
            recipient: principal(),
            key_id: [3; 32],
            envelope: vec![4],
        }],
    })
    .unwrap();
    let relation = ChangeRelation::try_new(ChangeRelation {
        policy_ref: p.clone(),
        relation_kind: ChangeRelationKind::Split,
        sources: vec![id(6)],
        results: vec![id(7)],
        provenance: vec![id(8)],
        creating_operation: id(4),
    })
    .unwrap();
    let resolution = ConflictResolution::try_new(ConflictResolution {
        policy_ref: p.clone(),
        conflict: id(9),
        resolved: typed(ObjectKind::Blob, 10),
        resolver: actor,
        device,
        resolution_kind: ResolutionKind::Manual,
        provenance: vec![typed(ObjectKind::Blob, 11)],
        wall_time: time(),
        signatures: vec![signature(SignaturePurpose::ConflictResolution)],
    })
    .unwrap();
    let fields = |purpose| EvidenceFields {
        policy_ref: p.clone(),
        target: typed(ObjectKind::ChangeRevision, 12),
        snapshot: id(13),
        ruleset: id(14),
        issuer: actor,
        device,
        outcome: EvidenceOutcome::Pass,
        constraints: vec![0xa0],
        related: vec![typed(ObjectKind::Snapshot, 15)],
        wall_time: time(),
        signatures: vec![signature(purpose)],
    };
    let review = ReviewEvidence::try_new(fields(SignaturePurpose::ReviewEvidence)).unwrap();
    let approval = ApprovalEvidence::try_new(fields(SignaturePurpose::ApprovalEvidence)).unwrap();
    let ci = CiEvidence::try_new(CiEvidence {
        fields: fields(SignaturePurpose::CiEvidence),
        check_name: "test".into(),
        runner_identity: id(2),
        build_provenance: Some(id(16)),
    })
    .unwrap();
    let decision =
        PolicyDecisionEvidence::try_new(fields(SignaturePurpose::PolicyDecisionEvidence)).unwrap();
    let projection_rules = ProjectionRules::try_new(ProjectionRules {
        policy_ref: p.clone(),
        version: 0,
        previous: None,
        rules: vec![ProjectionRule {
            rule_kind: ProjectionRuleKind::Include,
            parameters: vec![0xa0],
        }],
        default_fail: true,
    })
    .unwrap();
    let proof = ProjectionProof::try_new(ProjectionProof {
        policy_ref: p.clone(),
        algorithm: ProjectionProofAlgorithm::MerkleV0,
        source_snapshot: id(13),
        rules: id(14),
        audience_policy: id(1),
        projected_manifest: id(17),
        proof: vec![1],
    })
    .unwrap();
    let provenance = BuildProvenance::try_new(BuildProvenance {
        policy_ref: p.clone(),
        snapshot: id(13),
        ruleset: id(14),
        builder_identity: id(2),
        inputs: vec![typed(ObjectKind::Blob, 18)],
        outputs: vec![id(19)],
        reproducibility: vec![0xa0],
        wall_time: time(),
        signatures: vec![signature(SignaturePurpose::BuildProvenance)],
    })
    .unwrap();
    let artifact = Artifact::try_new(Artifact {
        policy_ref: p.clone(),
        artifact_kind: ArtifactKind::Binary,
        digest_algorithm: ArtifactDigestAlgorithm::Sha256,
        digest: vec![1; 32],
        byte_length: 1,
        locator: None,
        blob: Some(id(18)),
    })
    .unwrap();
    let payload = OperationPayload::try_new(OperationPayload {
        policy_ref: p.clone(),
        payload_kind: OperationPayloadKind::Inverse,
        references: vec![typed(ObjectKind::Snapshot, 13)],
        payload_schema: OperationPayloadSchema::CanonicalCborMapV0,
        canonical_payload: vec![0xa0],
    })
    .unwrap();
    let view = View::try_new(View {
        policy_ref: p.clone(),
        actor,
        device,
        policies: vec![id(1)],
        lines: vec![LineGeneration {
            line_id: LineId::from_bytes([1; 16]),
            generation: 0,
            state: id(5),
        }],
        projected_manifest: id(17),
        validity_constraints: vec![0xa0],
        signatures: vec![signature(SignaturePurpose::View)],
    })
    .unwrap();
    let migration = Migration::try_new(Migration {
        policy_ref: p.clone(),
        source_format: ObjectIdFormat::V0,
        target_format: ObjectIdFormat::V1,
        mappings: vec![IdMapping {
            old: id(20),
            new: id(21),
        }],
        tool_identity: id(2),
        wall_time: time(),
        signatures: vec![signature(SignaturePurpose::Migration)],
    })
    .unwrap();
    let ruleset = Ruleset::try_new(Ruleset {
        policy_ref: p,
        ruleset_kind: RulesetKind::Review,
        version: 0,
        previous: None,
        constraints: vec![0xa0],
        required_evidence_kinds: vec![ObjectKind::ReviewEvidence],
    })
    .unwrap();
    let mut out = Vec::new();
    macro_rules! add {
        ($name:literal,$v:expr) => {{
            let v = &$v;
            out.push(vector($name, v, None));
        }};
    }
    macro_rules! signed {
        ($name:literal,$v:expr) => {{
            let v = &$v;
            let u = v.unsigned_encode().unwrap();
            let pre = v.signing_preimage(&v.signatures()[0]).unwrap();
            out.push(vector($name, v, Some((&u, &pre))));
        }};
    }
    signed!("repository-root", root);
    signed!("identity", identity);
    signed!("group-membership", membership);
    add!("key-envelope-set", envelopes);
    add!("change-relation", relation);
    signed!("conflict-resolution", resolution);
    signed!("review-evidence", review);
    signed!("approval-evidence", approval);
    signed!("ci-evidence", ci);
    signed!("policy-decision-evidence", decision);
    add!("projection-rules", projection_rules);
    add!("projection-proof", proof);
    signed!("build-provenance", provenance);
    add!("artifact", artifact);
    add!("operation-payload", payload);
    signed!("view", view);
    signed!("migration", migration);
    add!("ruleset", ruleset);
    out
}

#[test]
fn extended_schema_vectors_are_frozen() {
    let actual = vectors();
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/extended-v0.json");
    if std::env::var_os("RGIT_UPDATE_VECTORS").is_some() {
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
        )
        .unwrap();
        return;
    }
    let expected: Vec<Vector> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn bootstrap_and_registry_negatives() {
    let invalid = Identity {
        policy_ref: Some(policy()),
        subject_kind: IdentitySubjectKind::Actor,
        subject: [1; 16],
        version: 0,
        previous: None,
        signing_keys: vec![],
        encryption_keys: vec![],
        issuer: ActorId::from_bytes([1; 16]),
        status: IdentityStatus::Active,
        activation_operation: None,
        not_after_operation: None,
        signatures: vec![signature(SignaturePurpose::Identity)],
    };
    assert!(Identity::try_new(invalid).is_err());
    assert!(
        Migration::try_new(Migration {
            policy_ref: policy(),
            source_format: ObjectIdFormat::V0,
            target_format: ObjectIdFormat::V0,
            mappings: vec![],
            tool_identity: id(1),
            wall_time: time(),
            signatures: vec![signature(SignaturePurpose::Migration)]
        })
        .is_err()
    );
    assert!(
        Ruleset::try_new(Ruleset {
            policy_ref: policy(),
            ruleset_kind: RulesetKind::Review,
            version: 0,
            previous: None,
            constraints: vec![],
            required_evidence_kinds: vec![ObjectKind::Blob]
        })
        .is_err()
    );
    assert!(ObjectKind::try_from(32).is_err());
}

#[test]
fn linked_bootstrap_graph_is_acyclic_and_hash_linked() {
    let actor = ActorId::from_bytes([2; 16]);
    let device = DeviceId::from_bytes([4; 16]);
    let envelopes = KeyEnvelopeSet::try_new(KeyEnvelopeSet {
        policy_ref: None,
        epoch: 0,
        suite: KeyEnvelopeSuite::X25519HkdfSha256Aes256Gcm,
        recipients: vec![RecipientEnvelope {
            recipient: principal(),
            key_id: [3; 32],
            envelope: vec![4],
        }],
    })
    .unwrap();
    let envelope_id = envelopes.id(HashAlgorithm::Sha256).unwrap();
    let policy = Policy {
        policy_ref: None,
        policy_id: PolicyId::from_bytes([1; 16]),
        version_sequence: 0,
        previous_version: None,
        principals: vec![principal()],
        grants: vec![],
        redaction_mode: RedactionMode::Omit,
        derivation_rule: DerivationRule::NoDerivation,
        declassification_requirements: vec![],
        key_epoch: 0,
        key_envelope_set: envelope_id,
        administrators: vec![actor],
        activation_constraints: vec![0xa0],
        signatures: vec![signature(SignaturePurpose::Policy)],
    };
    let policy_id = policy.id(HashAlgorithm::Sha256).unwrap();
    let policy_ref = PolicyRef {
        policy_id: policy.policy_id,
        version: policy_id.clone(),
    };
    let identity = Identity::try_new(Identity {
        policy_ref: None,
        subject_kind: IdentitySubjectKind::Actor,
        subject: [2; 16],
        version: 0,
        previous: None,
        signing_keys: vec![PublicKeyRecord {
            algorithm: PublicKeyAlgorithm::Ed25519,
            key_id: [3; 32],
            public_key: vec![4; 32],
            not_before: None,
            not_after: None,
        }],
        encryption_keys: vec![],
        issuer: actor,
        status: IdentityStatus::Active,
        activation_operation: None,
        not_after_operation: None,
        signatures: vec![signature(SignaturePurpose::Identity)],
    })
    .unwrap();
    let line_id = LineId::from_bytes([8; 16]);
    let declaration = LineAdvanceDeclaration {
        policy_ref: policy_ref.clone(),
        line_id,
        display_name: "main".into(),
        head_snapshot: id(30),
        generation: 0,
        previous_state: None,
        integration_policy: policy_ref.clone(),
        approval_policy: policy_ref.clone(),
        release_policy: policy_ref.clone(),
        visibility_policy: policy_ref.clone(),
    };
    let operation = Operation {
        policy_ref: policy_ref.clone(),
        parents: vec![],
        actor,
        device,
        logical_time: 0,
        wall_time: time(),
        actions: vec![OperationAction::LineAdvance(Box::new(declaration.clone()))],
        inverse_payloads: vec![],
        public_envelope: None,
        private_payload: None,
        signature: signature(SignaturePurpose::Operation),
        client_implementation: "rgit-test".into(),
    };
    let operation_id = operation.id(HashAlgorithm::Sha256).unwrap();
    let line = LineState {
        policy_ref: policy_ref.clone(),
        line_id,
        display_name: "main".into(),
        head_snapshot: id(30),
        generation: 0,
        previous_state: None,
        integration_policy: policy_ref.clone(),
        approval_policy: policy_ref.clone(),
        release_policy: policy_ref.clone(),
        visibility_policy: policy_ref,
        transaction_operation: operation_id,
        signature: signature(SignaturePurpose::LineState),
    };
    let root = RepositoryRoot::try_new(RepositoryRoot {
        repository_id: RepositoryId::from_bytes([9; 16]),
        root_policy: policy_id,
        trusted_identities: vec![identity.id(HashAlgorithm::Sha256).unwrap()],
        bootstrap_key_envelope_set: envelopes.id(HashAlgorithm::Sha256).unwrap(),
        genesis_operation: operation.id(HashAlgorithm::Sha256).unwrap(),
        initial_line_states: vec![line.id(HashAlgorithm::Sha256).unwrap()],
        filesystem_profile: FilesystemProfile::Portable,
        signatures: vec![signature(SignaturePurpose::RepositoryRoot)],
    })
    .unwrap();
    let identities = [identity];
    let lines = [line];
    let graph = BootstrapGraph {
        root: &root,
        root_policy: &policy,
        identities: &identities,
        key_envelope_set: &envelopes,
        genesis_operation: &operation,
        initial_line_states: &lines,
    };
    graph.validate(HashAlgorithm::Sha256).unwrap();
    let actual = BootstrapFixture {
        root_id: root.id(HashAlgorithm::Sha256).unwrap().to_string(),
        root_cbor_hex: hex::encode(root.encode().unwrap()),
        root_policy_id: policy.id(HashAlgorithm::Sha256).unwrap().to_string(),
        identity_ids: identities
            .iter()
            .map(|value| value.id(HashAlgorithm::Sha256).unwrap().to_string())
            .collect(),
        key_envelope_set_id: envelopes.id(HashAlgorithm::Sha256).unwrap().to_string(),
        genesis_operation_id: operation.id(HashAlgorithm::Sha256).unwrap().to_string(),
        initial_line_state_ids: lines
            .iter()
            .map(|value| value.id(HashAlgorithm::Sha256).unwrap().to_string())
            .collect(),
        dependency_order: vec![
            "identity-and-key-envelope".into(),
            "root-policy".into(),
            "genesis-operation".into(),
            "genesis-line-state".into(),
            "repository-root".into(),
        ],
    };
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/bootstrap-v0.json");
    if std::env::var_os("RGIT_UPDATE_VECTORS").is_some() {
        std::fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
        )
        .unwrap();
    } else {
        let expected: BootstrapFixture =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(actual, expected);
    }
    let mut bad_root = root.clone();
    bad_root.genesis_operation = id(99);
    assert!(
        BootstrapGraph {
            root: &bad_root,
            root_policy: &policy,
            identities: &identities,
            key_envelope_set: &envelopes,
            genesis_operation: &operation,
            initial_line_states: &lines,
        }
        .validate(HashAlgorithm::Sha256)
        .is_err()
    );
}

fn decode_value(value: &Value) -> Result<AnyObject, DecodeObjectError> {
    AnyObject::decode(&value.encode().unwrap(), CanonicalLimits::metadata())
}

#[test]
fn every_extended_kind_rejects_structural_schema_mutations() {
    for fixture in vectors() {
        let bytes = hex::decode(&fixture.canonical_cbor_hex).unwrap();
        let value = decode_canonical(&bytes, CanonicalLimits::metadata()).unwrap();
        let Value::Map(map) = value else {
            unreachable!()
        };
        let required: &[u64] = match fixture.kind {
            14 => &[0, 1, 3, 4, 5, 6, 7, 8, 9, 10],
            15 => &[0, 1, 3, 4, 5, 7, 8, 9, 10, 13],
            16 => &[0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 12],
            17 => &[0, 1, 3, 4, 5],
            18 => &[0, 1, 2, 3, 4, 5, 6, 7],
            19 => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
            20 | 21 | 23 => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
            22 => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
            24 => &[0, 1, 2, 3, 5, 6],
            25 => &[0, 1, 2, 3, 4, 5, 6, 7, 8],
            26 => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
            27 => &[0, 1, 2, 3, 4, 5, 6],
            28 => &[0, 1, 2, 3, 4, 5, 6],
            29 => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
            30 => &[0, 1, 2, 3, 4, 5, 6, 7, 8],
            31 => &[0, 1, 2, 3, 4, 6, 7],
            _ => unreachable!(),
        };
        for required in required {
            let mut missing = map.clone();
            missing.retain(|(key, _)| key != required);
            assert!(
                decode_value(&Value::Map(missing)).is_err(),
                "{} accepted missing field {}",
                fixture.name,
                required
            );
            let mut wrong = map.clone();
            wrong.iter_mut().find(|(key, _)| key == required).unwrap().1 = Value::Null;
            assert!(
                decode_value(&Value::Map(wrong)).is_err(),
                "{} accepted wrong type for field {}",
                fixture.name,
                required
            );
        }

        let mut unknown = map.clone();
        unknown.push((99, Value::Unsigned(0)));
        unknown.sort_by_key(|(key, _)| *key);
        assert!(
            decode_value(&Value::Map(unknown)).is_err(),
            "{} accepted unknown field",
            fixture.name
        );
    }
}

#[test]
fn closed_registries_sets_signatures_and_payloads_reject_mutations() {
    for fixture in vectors() {
        let bytes = hex::decode(&fixture.canonical_cbor_hex).unwrap();
        let Value::Map(map) = decode_canonical(&bytes, CanonicalLimits::metadata()).unwrap() else {
            unreachable!()
        };
        let enum_field = match fixture.kind {
            14 => Some(9),
            15 => Some(3),
            16 => Some(8),
            17 => Some(4),
            18 => Some(3),
            19 => Some(7),
            20..=23 => Some(8),
            24 => None,
            25 => Some(3),
            26 => None,
            27 => Some(3),
            28 => Some(3),
            29 => None,
            30 => Some(3),
            31 => Some(3),
            _ => None,
        };
        if let Some(field) = enum_field {
            let mut changed = map.clone();
            changed.iter_mut().find(|(key, _)| *key == field).unwrap().1 =
                Value::Unsigned(u64::MAX);
            assert!(
                decode_value(&Value::Map(changed)).is_err(),
                "{} accepted unknown enum",
                fixture.name
            );
        }
        let signature_field = match fixture.kind {
            14 => Some((10, SignaturePurpose::RepositoryRoot)),
            15 => Some((13, SignaturePurpose::Identity)),
            16 => Some((12, SignaturePurpose::GroupMembership)),
            19 => Some((10, SignaturePurpose::ConflictResolution)),
            20 => Some((12, SignaturePurpose::ReviewEvidence)),
            21 => Some((12, SignaturePurpose::ApprovalEvidence)),
            22 => Some((12, SignaturePurpose::CiEvidence)),
            23 => Some((12, SignaturePurpose::PolicyDecisionEvidence)),
            26 => Some((10, SignaturePurpose::BuildProvenance)),
            29 => Some((9, SignaturePurpose::View)),
            30 => Some((8, SignaturePurpose::Migration)),
            _ => None,
        };
        if let Some((field, _)) = signature_field {
            let mut empty = map.clone();
            empty.iter_mut().find(|(key, _)| *key == field).unwrap().1 = Value::Array(vec![]);
            assert!(
                decode_value(&Value::Map(empty)).is_err(),
                "{} accepted no signatures",
                fixture.name
            );
            let mut wrong = map.clone();
            let Value::Array(values) =
                &mut wrong.iter_mut().find(|(key, _)| *key == field).unwrap().1
            else {
                unreachable!()
            };
            let Value::Map(record) = &mut values[0] else {
                unreachable!()
            };
            record.iter_mut().find(|(key, _)| *key == 4).unwrap().1 = Value::Unsigned(0);
            assert!(
                decode_value(&Value::Map(wrong)).is_err(),
                "{} accepted wrong signature purpose",
                fixture.name
            );
            let mut size = map.clone();
            let Value::Array(values) =
                &mut size.iter_mut().find(|(key, _)| *key == field).unwrap().1
            else {
                unreachable!()
            };
            let Value::Map(record) = &mut values[0] else {
                unreachable!()
            };
            record.iter_mut().find(|(key, _)| *key == 3).unwrap().1 = Value::Bytes(vec![1; 63]);
            assert!(
                decode_value(&Value::Map(size)).is_err(),
                "{} accepted wrong signature size",
                fixture.name
            );
        }
    }
    let payload = OperationPayload {
        policy_ref: policy(),
        payload_kind: OperationPayloadKind::Inverse,
        references: vec![],
        payload_schema: OperationPayloadSchema::CanonicalCborMapV0,
        canonical_payload: vec![0xff],
    };
    assert!(OperationPayload::try_new(payload).is_err());
    let view = View {
        policy_ref: policy(),
        actor: ActorId::from_bytes([1; 16]),
        device: DeviceId::from_bytes([2; 16]),
        policies: vec![],
        lines: vec![
            LineGeneration {
                line_id: LineId::from_bytes([3; 16]),
                generation: 0,
                state: id(1),
            },
            LineGeneration {
                line_id: LineId::from_bytes([3; 16]),
                generation: 1,
                state: id(2),
            },
        ],
        projected_manifest: id(3),
        validity_constraints: vec![0xa0],
        signatures: vec![signature(SignaturePurpose::View)],
    };
    assert!(View::try_new(view).is_err());

    let nonbootstrap = |policy_ref, previous, activation_operation| Identity {
        policy_ref,
        subject_kind: IdentitySubjectKind::Actor,
        subject: [1; 16],
        version: 1,
        previous,
        signing_keys: vec![PublicKeyRecord {
            algorithm: PublicKeyAlgorithm::Ed25519,
            key_id: [3; 32],
            public_key: vec![4; 32],
            not_before: None,
            not_after: None,
        }],
        encryption_keys: vec![],
        issuer: ActorId::from_bytes([1; 16]),
        status: IdentityStatus::Active,
        activation_operation,
        not_after_operation: None,
        signatures: vec![signature(SignaturePurpose::Identity)],
    };
    assert!(Identity::try_new(nonbootstrap(None, Some(id(1)), Some(id(2)))).is_err());
    assert!(Identity::try_new(nonbootstrap(Some(policy()), None, Some(id(2)))).is_err());
    assert!(Identity::try_new(nonbootstrap(Some(policy()), Some(id(1)), None)).is_err());
    let duplicate_key = Identity {
        encryption_keys: vec![PublicKeyRecord {
            algorithm: PublicKeyAlgorithm::X25519,
            key_id: [3; 32],
            public_key: vec![5; 32],
            not_before: None,
            not_after: None,
        }],
        ..nonbootstrap(Some(policy()), Some(id(1)), Some(id(2)))
    };
    assert!(Identity::try_new(duplicate_key).is_err());
    for length in [31, 33] {
        let wrong_length = Identity {
            signing_keys: vec![PublicKeyRecord {
                algorithm: PublicKeyAlgorithm::Ed25519,
                key_id: [4; 32],
                public_key: vec![5; length],
                not_before: None,
                not_after: None,
            }],
            ..nonbootstrap(Some(policy()), Some(id(1)), Some(id(2)))
        };
        assert!(Identity::try_new(wrong_length).is_err());
    }
    let identity_fixture = vectors()
        .into_iter()
        .find(|vector| vector.kind == 15)
        .unwrap();
    let Value::Map(identity_map) = decode_canonical(
        &hex::decode(identity_fixture.canonical_cbor_hex).unwrap(),
        CanonicalLimits::metadata(),
    )
    .unwrap() else {
        unreachable!()
    };
    for length in [31, 33] {
        let mut changed = identity_map.clone();
        let Value::Array(keys) = &mut changed.iter_mut().find(|(key, _)| *key == 7).unwrap().1
        else {
            unreachable!()
        };
        let Value::Map(key) = &mut keys[0] else {
            unreachable!()
        };
        key.iter_mut().find(|(field, _)| *field == 2).unwrap().1 = Value::Bytes(vec![5; length]);
        assert!(decode_value(&Value::Map(changed)).is_err());
    }
}

#[test]
fn extended_reference_kinds_and_set_ordering_fail_closed() {
    for fixture in vectors() {
        let bytes = hex::decode(&fixture.canonical_cbor_hex).unwrap();
        let Value::Map(map) = decode_canonical(&bytes, CanonicalLimits::metadata()).unwrap() else {
            unreachable!()
        };
        let typed_array = match fixture.kind {
            19 => Some(8),
            20..=23 => Some(10),
            26 => Some(6),
            28 => Some(4),
            _ => None,
        };
        if let Some(field) = typed_array {
            let mut changed = map.clone();
            let Value::Array(values) =
                &mut changed.iter_mut().find(|(key, _)| *key == field).unwrap().1
            else {
                unreachable!()
            };
            if let Some(Value::Map(reference)) = values.first_mut() {
                reference.iter_mut().find(|(key, _)| *key == 0).unwrap().1 = Value::Unsigned(99);
                assert!(
                    decode_value(&Value::Map(changed)).is_err(),
                    "{} accepted unknown typed reference kind",
                    fixture.name
                );
            }
        }
        let duplicate_field = match fixture.kind {
            14 => Some(5),
            15 => Some(7),
            17 => Some(5),
            18 => Some(4),
            19 => Some(8),
            20..=23 => Some(10),
            26 => Some(6),
            28 => Some(4),
            29 => Some(5),
            30 => Some(5),
            _ => None,
        };
        if let Some(field) = duplicate_field {
            let mut changed = map;
            let Value::Array(values) =
                &mut changed.iter_mut().find(|(key, _)| *key == field).unwrap().1
            else {
                unreachable!()
            };
            if let Some(first) = values.first().cloned() {
                values.push(first);
                assert!(
                    decode_value(&Value::Map(changed)).is_err(),
                    "{} accepted duplicate set member",
                    fixture.name
                );
            }
        }
    }
}
