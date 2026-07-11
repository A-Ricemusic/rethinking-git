use rgit_graph::{GenerationError, GraphError, GraphIndex, GraphNode, next_generation};
use rgit_objects::{
    ActorId, CanonicalObject, ChangeId, Chunk, DerivationRule, DeviceId, HashAlgorithm,
    KeyEnvelopeSet, KeyEnvelopeSuite, LineId, LineState, Marker, MarkerKind, ObjectId, ObjectKind,
    Operation, OperationAction, Policy, PolicyId, PolicyRef, Principal, PrincipalKind,
    RecipientEnvelope, RedactionMode, ReferenceEdge, ReferenceRole, Signature, SignatureAlgorithm,
    SignaturePurpose, Snapshot, TypedObjectRef, Value, WallTime,
};
use rgit_store::{
    ClosureError, ExpectedRef, MemoryStore, ObjectPresence, Publication, PublicationCandidate,
    PublicationObject, PublicationValidator, PutOutcome, RefUpdate, ReferenceKey, ReferenceState,
    Store, StoreError,
};

fn allow(_: &PublicationCandidate<'_>) -> Result<(), StoreError> {
    Ok(())
}
fn deny(_: &PublicationCandidate<'_>) -> Result<(), StoreError> {
    Err(StoreError::PublicationDenied)
}

struct ReentrantValidator<'a>(&'a MemoryStore);
impl PublicationValidator for ReentrantValidator<'_> {
    fn validate(&self, candidate: &PublicationCandidate<'_>) -> Result<(), StoreError> {
        let _ = self.0.presence(&candidate.publication().operation);
        Ok(())
    }
}

fn fake_id(byte: u8) -> ObjectId {
    let mut bytes = vec![0, HashAlgorithm::Sha256 as u8, 32];
    bytes.extend([byte; 32]);
    ObjectId::from_bytes(&bytes).expect("test ID")
}

fn encoded<T: CanonicalObject>(value: &T) -> (ObjectId, Vec<u8>) {
    (
        value.id(HashAlgorithm::Sha256).expect("ID"),
        value.encode().expect("encoding"),
    )
}

fn signature(seed: u8, purpose: SignaturePurpose) -> Signature {
    Signature::new(
        SignatureAlgorithm::Ed25519,
        ActorId::from_bytes([seed; 16]),
        [seed; 32],
        [seed; 64],
        purpose,
    )
    .expect("signature")
}

fn envelope(seed: u8) -> KeyEnvelopeSet {
    KeyEnvelopeSet {
        policy_ref: None,
        epoch: 0,
        suite: KeyEnvelopeSuite::X25519HkdfSha256Aes256Gcm,
        recipients: vec![RecipientEnvelope {
            recipient: Principal {
                kind: PrincipalKind::Actor,
                identifier: vec![seed],
            },
            key_id: [seed; 32],
            envelope: vec![seed],
        }],
    }
}

fn put<T: CanonicalObject>(store: &MemoryStore, value: &T) -> ObjectId {
    let (id, bytes) = encoded(value);
    assert_eq!(store.put(id.clone(), bytes), Ok(PutOutcome::New));
    id
}

fn bootstrap_policy(store: &MemoryStore, seed: u8) -> PolicyRef {
    let envelopes = put(store, &envelope(seed));
    let policy_id = PolicyId::from_bytes([seed; 16]);
    let policy = Policy {
        policy_ref: None,
        policy_id,
        version_sequence: 0,
        previous_version: None,
        principals: vec![Principal {
            kind: PrincipalKind::Actor,
            identifier: vec![seed],
        }],
        grants: Vec::new(),
        redaction_mode: RedactionMode::Omit,
        derivation_rule: DerivationRule::NoDerivation,
        declassification_requirements: Vec::new(),
        key_epoch: 0,
        key_envelope_set: envelopes,
        administrators: vec![ActorId::from_bytes([seed; 16])],
        activation_constraints: Value::Map(Vec::new()).encode().expect("constraints"),
        signatures: vec![signature(seed, SignaturePurpose::Policy)],
    };
    let version = put(store, &policy);
    PolicyRef { policy_id, version }
}

fn operation(
    policy_ref: PolicyRef,
    parents: Vec<ObjectId>,
    actions: Vec<OperationAction>,
    seed: u8,
) -> Operation {
    Operation {
        policy_ref,
        parents,
        actor: ActorId::from_bytes([seed; 16]),
        device: DeviceId::from_bytes([seed.wrapping_add(1); 16]),
        logical_time: u64::from(seed),
        wall_time: WallTime {
            utc_seconds: i64::from(seed),
            offset_seconds: 0,
        },
        actions,
        inverse_payloads: Vec::new(),
        public_envelope: None,
        private_payload: None,
        signature: signature(seed, SignaturePurpose::Operation),
        client_implementation: "rgit-store-contract".into(),
    }
}

fn snapshot(policy_ref: PolicyRef, parents: Vec<ObjectId>, seed: u8) -> Snapshot {
    Snapshot {
        policy_ref,
        root_manifest: fake_id(33),
        parents,
        change_id: ChangeId::from_bytes([seed; 16]),
        author: ActorId::from_bytes([seed; 16]),
        device: DeviceId::from_bytes([seed.wrapping_add(1); 16]),
        logical_time: u64::from(seed),
        wall_time: WallTime {
            utc_seconds: i64::from(seed),
            offset_seconds: 0,
        },
        message_blob: None,
    }
}

fn publish_operation_head(
    store: &MemoryStore,
    operation: &Operation,
    expected: ExpectedRef,
) -> Result<ReferenceState, StoreError> {
    let (id, bytes) = encoded(operation);
    store
        .publish(
            Publication {
                objects: vec![PublicationObject {
                    id: id.clone(),
                    bytes,
                }],
                updates: vec![RefUpdate {
                    key: ReferenceKey::OperationHead,
                    expected,
                    target: id.clone(),
                }],
                operation: id,
            },
            &allow,
        )
        .map(|states| states.into_iter().next().expect("one state"))
}

#[test]
fn put_is_verified_idempotent_and_deduplicated() {
    let store = MemoryStore::new();
    let policy_ref = bootstrap_policy(&store, 7);
    let chunk = Chunk {
        policy_ref,
        bytes: b"hello".to_vec(),
    };
    let (id, bytes) = encoded(&chunk);
    assert_eq!(store.put(id.clone(), bytes.clone()), Ok(PutOutcome::New));
    assert_eq!(store.put(id.clone(), bytes), Ok(PutOutcome::AlreadyPresent));
    assert_eq!(store.presence(&id), Some(ObjectPresence::Present));
}

#[test]
fn put_rejects_digest_kind_and_schema_corruption() {
    let store = MemoryStore::new();
    let policy_ref = bootstrap_policy(&store, 7);
    let (id, mut bytes) = encoded(&Chunk {
        policy_ref,
        bytes: vec![1],
    });
    *bytes.last_mut().expect("encoding") ^= 1;
    assert!(matches!(
        store.put(id.clone(), bytes),
        Err(StoreError::InvalidObject(_))
    ));
    let unknown = Value::Map(vec![(0, Value::Unsigned(99)), (1, Value::Unsigned(0))])
        .encode()
        .expect("value");
    assert!(matches!(
        store.put(id.clone(), unknown),
        Err(StoreError::InvalidObject(
            rgit_objects::DecodeObjectError::Kind(99)
        ))
    ));
    let schema = Value::Map(vec![(0, Value::Unsigned(1)), (1, Value::Unsigned(1))])
        .encode()
        .expect("value");
    assert!(matches!(
        store.put(id, schema),
        Err(StoreError::InvalidObject(
            rgit_objects::DecodeObjectError::Schema(1)
        ))
    ));
}

#[test]
fn operation_head_cas_checks_full_state_and_prevents_aba() {
    let store = MemoryStore::new();
    let policy = bootstrap_policy(&store, 2);
    let first = publish_operation_head(
        &store,
        &operation(policy.clone(), Vec::new(), Vec::new(), 3),
        ExpectedRef::Absent,
    )
    .expect("first");
    let second = publish_operation_head(
        &store,
        &operation(policy.clone(), vec![first.target.clone()], Vec::new(), 4),
        ExpectedRef::Exact(first.clone()),
    )
    .expect("second");
    assert_eq!(second.generation, 1);
    let stale = ExpectedRef::Exact(first);
    let third = operation(policy, vec![second.target], Vec::new(), 5);
    assert_eq!(
        publish_operation_head(&store, &third, stale),
        Err(StoreError::ReferenceConflict)
    );
}

#[test]
fn compare_and_swap_uses_structural_and_pluggable_validation() {
    let store = MemoryStore::new();
    let policy = bootstrap_policy(&store, 6);
    let operation = operation(policy, Vec::new(), Vec::new(), 7);
    let id = put(&store, &operation);
    assert_eq!(
        store.compare_and_swap(
            ReferenceKey::OperationHead,
            ExpectedRef::Absent,
            id.clone(),
            id,
            &deny
        ),
        Err(StoreError::PublicationDenied)
    );
    assert_eq!(store.reference(&ReferenceKey::OperationHead), None);
}

#[test]
fn validator_can_safely_reenter_read_apis() {
    let store = MemoryStore::new();
    let policy = bootstrap_policy(&store, 24);
    let operation = operation(policy, Vec::new(), Vec::new(), 25);
    let (id, bytes) = encoded(&operation);
    let publication = Publication {
        objects: vec![PublicationObject {
            id: id.clone(),
            bytes,
        }],
        updates: vec![RefUpdate {
            key: ReferenceKey::OperationHead,
            expected: ExpectedRef::Absent,
            target: id.clone(),
        }],
        operation: id,
    };
    assert!(
        store
            .publish(publication, &ReentrantValidator(&store))
            .is_ok()
    );
}

#[test]
fn closure_checks_missing_promised_quarantine_and_expected_kind() {
    let store = MemoryStore::new();
    let id = put(&store, &envelope(8));
    let root = |id: ObjectId, expected_kind| ReferenceEdge {
        role: ReferenceRole::MarkerTarget,
        expected_kind,
        id,
    };
    assert_eq!(
        store
            .closure(&[root(id.clone(), Some(ObjectKind::KeyEnvelopeSet))])
            .expect("closure")
            .objects()
            .len(),
        1
    );
    assert!(matches!(
        store.closure(&[root(id.clone(), Some(ObjectKind::Blob))]),
        Err(StoreError::Closure(ClosureError::WrongKind { .. }))
    ));
    assert_eq!(
        store.closure(&[root(fake_id(44), None)]),
        Err(StoreError::Closure(ClosureError::Missing))
    );
    let promised = fake_id(45);
    store.mark_promised(promised.clone()).expect("promise");
    assert_eq!(
        store.closure(&[root(promised, None)]),
        Err(StoreError::Closure(ClosureError::Promised))
    );
    store.quarantine(&id).expect("quarantine");
    assert_eq!(
        store.closure(&[root(id, None)]),
        Err(StoreError::Closure(ClosureError::Quarantined))
    );
}

#[test]
fn parent_diamonds_have_generations_and_reachability() {
    let store = MemoryStore::new();
    let policy = bootstrap_policy(&store, 9);
    let a = put(&store, &snapshot(policy.clone(), Vec::new(), 1));
    let b = put(&store, &snapshot(policy.clone(), vec![a.clone()], 2));
    let c = put(&store, &snapshot(policy.clone(), vec![a.clone()], 3));
    let d = put(&store, &snapshot(policy, vec![b.clone(), c.clone()], 4));
    assert_eq!(store.generation(&a), Ok(0));
    assert_eq!(store.generation(&d), Ok(2));
    assert_eq!(store.is_reachable(&a, &d), Ok(true));
    assert_eq!(store.is_reachable(&b, &c), Ok(false));
}

#[test]
fn graph_dependency_rejects_cycles_and_generation_overflow() {
    assert!(matches!(
        GraphIndex::build([
            GraphNode::new("a", vec!["b"]),
            GraphNode::new("b", vec!["a"])
        ]),
        Err(GraphError::Cycle { .. })
    ));
    assert_eq!(next_generation([u64::MAX]), Err(GenerationError::Overflow));
}

#[test]
fn publication_rejects_wrong_kind_and_rolls_back() {
    let store = MemoryStore::new();
    let policy = bootstrap_policy(&store, 10);
    let operation = operation(policy, Vec::new(), Vec::new(), 11);
    let (id, bytes) = encoded(&operation);
    let publication = Publication {
        objects: vec![PublicationObject {
            id: id.clone(),
            bytes,
        }],
        updates: vec![RefUpdate {
            key: ReferenceKey::Marker([1; 16]),
            expected: ExpectedRef::Absent,
            target: id.clone(),
        }],
        operation: id.clone(),
    };
    assert_eq!(
        store.publish(publication, &allow),
        Err(StoreError::ReferenceKind)
    );
    assert_eq!(store.presence(&id), None);
}

#[test]
fn line_publication_requires_matching_line_advance_action() {
    let store = MemoryStore::new();
    let policy = bootstrap_policy(&store, 12);
    let operation = operation(policy.clone(), Vec::new(), Vec::new(), 13);
    let (operation_id, operation_bytes) = encoded(&operation);
    let line_id = LineId::from_bytes([14; 16]);
    let line = LineState {
        policy_ref: policy.clone(),
        line_id,
        display_name: "main".into(),
        head_snapshot: fake_id(15),
        generation: 0,
        previous_state: None,
        integration_policy: policy.clone(),
        approval_policy: policy.clone(),
        release_policy: policy.clone(),
        visibility_policy: policy,
        transaction_operation: operation_id.clone(),
        signature: signature(14, SignaturePurpose::LineState),
    };
    let (line_state, line_bytes) = encoded(&line);
    let publication = Publication {
        objects: vec![
            PublicationObject {
                id: operation_id.clone(),
                bytes: operation_bytes,
            },
            PublicationObject {
                id: line_state.clone(),
                bytes: line_bytes,
            },
        ],
        updates: vec![RefUpdate {
            key: ReferenceKey::Line(line_id),
            expected: ExpectedRef::Absent,
            target: line_state.clone(),
        }],
        operation: operation_id,
    };
    assert_eq!(
        store.publish(publication, &allow),
        Err(StoreError::OperationMismatch)
    );
    assert_eq!(store.presence(&line_state), None);
}

#[test]
fn publication_rejects_missing_transitive_target_and_rolls_back() {
    let store = MemoryStore::new();
    let policy = bootstrap_policy(&store, 16);
    let missing = fake_id(90);
    let marker = Marker {
        policy_ref: policy.clone(),
        marker_kind: MarkerKind::Bookmark,
        target: TypedObjectRef {
            kind: ObjectKind::Blob,
            id: missing,
        },
        issuer: ActorId::from_bytes([16; 16]),
        issue_time: WallTime {
            utc_seconds: 0,
            offset_seconds: 0,
        },
        typed_payload: Vec::new(),
        signature: signature(16, SignaturePurpose::Marker),
    };
    let (marker_id, marker_bytes) = encoded(&marker);
    let action = OperationAction::Transition {
        before: None,
        after: Some(TypedObjectRef {
            kind: ObjectKind::Marker,
            id: marker_id.clone(),
        }),
    };
    let operation = operation(policy, Vec::new(), vec![action], 17);
    let (operation_id, operation_bytes) = encoded(&operation);
    let publication = Publication {
        objects: vec![
            PublicationObject {
                id: marker_id.clone(),
                bytes: marker_bytes,
            },
            PublicationObject {
                id: operation_id.clone(),
                bytes: operation_bytes,
            },
        ],
        updates: vec![RefUpdate {
            key: ReferenceKey::Marker([2; 16]),
            expected: ExpectedRef::Absent,
            target: marker_id.clone(),
        }],
        operation: operation_id,
    };
    assert_eq!(
        store.publish(publication, &allow),
        Err(StoreError::Closure(ClosureError::Missing))
    );
    assert_eq!(store.presence(&marker_id), None);
}

#[test]
fn unrelated_partial_graph_does_not_block_publication() {
    let store = MemoryStore::new();
    let policy = bootstrap_policy(&store, 18);
    put(&store, &snapshot(policy.clone(), vec![fake_id(91)], 19));
    let state = publish_operation_head(
        &store,
        &operation(policy, Vec::new(), Vec::new(), 20),
        ExpectedRef::Absent,
    )
    .expect("publication");
    assert_eq!(state.generation, 0);
}

#[test]
fn promised_targets_and_validator_denials_roll_back() {
    let store = MemoryStore::new();
    let policy = bootstrap_policy(&store, 21);
    let promised = fake_id(92);
    store.mark_promised(promised.clone()).expect("promise");
    let action = OperationAction::Transition {
        before: None,
        after: Some(TypedObjectRef {
            kind: ObjectKind::Marker,
            id: promised.clone(),
        }),
    };
    let operation_id = put(
        &store,
        &operation(policy.clone(), Vec::new(), vec![action], 22),
    );
    let publication = Publication {
        objects: Vec::new(),
        updates: vec![RefUpdate {
            key: ReferenceKey::Marker([3; 16]),
            expected: ExpectedRef::Absent,
            target: promised,
        }],
        operation: operation_id,
    };
    assert_eq!(
        store.publish(publication, &allow),
        Err(StoreError::Promised)
    );

    let operation = operation(policy, Vec::new(), Vec::new(), 23);
    let (id, bytes) = encoded(&operation);
    let denied = Publication {
        objects: vec![PublicationObject {
            id: id.clone(),
            bytes,
        }],
        updates: vec![RefUpdate {
            key: ReferenceKey::OperationHead,
            expected: ExpectedRef::Absent,
            target: id.clone(),
        }],
        operation: id.clone(),
    };
    assert_eq!(
        store.publish(denied, &deny),
        Err(StoreError::PublicationDenied)
    );
    assert_eq!(store.presence(&id), None);
}

#[test]
fn debug_and_display_are_redacted() {
    let secret_id = fake_id(88);
    let publication = Publication {
        objects: vec![PublicationObject {
            id: secret_id.clone(),
            bytes: b"secret payload".to_vec(),
        }],
        updates: vec![RefUpdate {
            key: ReferenceKey::Marker([0x55; 16]),
            expected: ExpectedRef::Exact(ReferenceState {
                target: secret_id.clone(),
                generation: 8,
                operation: secret_id.clone(),
            }),
            target: secret_id.clone(),
        }],
        operation: secret_id.clone(),
    };
    let debug = format!("{publication:?}");
    assert!(!debug.contains(&secret_id.to_string()));
    assert!(!debug.contains("secret payload"));
    assert!(!debug.contains("55555555"));
    let error = MemoryStore::new()
        .get(&secret_id)
        .expect_err("missing")
        .to_string();
    assert!(!error.contains(&secret_id.to_string()));
}
