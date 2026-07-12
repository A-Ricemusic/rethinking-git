use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Mutex, MutexGuard},
};

use rgit_graph::{GraphIndex, GraphNode};
use rgit_objects::{
    AnyObject, CanonicalLimits, ObjectId, ObjectKind, ReferenceEdge, ReferenceRole, Value,
};

use crate::{
    Closure, ClosureError, ExpectedRef, ObjectPresence, Publication, PublicationCandidate,
    PublicationValidator, PutOutcome, ReferenceKey, ReferenceState, Store, StoreError,
    StoredObject,
};

#[derive(Clone, Default)]
pub(crate) struct MemorySnapshot {
    pub(crate) objects: BTreeMap<ObjectId, StoredObject>,
    pub(crate) promised: BTreeSet<ObjectId>,
    pub(crate) quarantined: BTreeSet<ObjectId>,
    pub(crate) references: BTreeMap<ReferenceKey, ReferenceState>,
    pub(crate) revision: u64,
}

/// Thread-safe in-memory reference backend used by tests and ephemeral clients.
#[derive(Default)]
pub struct MemoryStore {
    state: Mutex<MemorySnapshot>,
}

impl MemoryStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_snapshot(snapshot: MemorySnapshot) -> Self {
        Self {
            state: Mutex::new(snapshot),
        }
    }

    pub(crate) fn snapshot(&self) -> MemorySnapshot {
        self.lock().clone()
    }

    pub(crate) fn replace_if_revision(
        &self,
        expected_revision: u64,
        replacement: MemorySnapshot,
    ) -> Result<(), StoreError> {
        let mut state = self.lock();
        if state.revision != expected_revision {
            return Err(StoreError::ReferenceConflict);
        }
        *state = replacement;
        Ok(())
    }

    pub(crate) fn replace(&self, replacement: MemorySnapshot) {
        *self.lock() = replacement;
    }

    fn lock(&self) -> MutexGuard<'_, MemorySnapshot> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn decode(id: &ObjectId, bytes: &[u8]) -> Result<StoredObject, StoreError> {
        let object = AnyObject::decode_verified(bytes, id, CanonicalLimits::bulk())?;
        Ok(StoredObject::new(id.clone(), bytes.to_vec(), object)?)
    }

    fn visible_object<'a>(
        state: &'a MemorySnapshot,
        id: &ObjectId,
    ) -> Result<&'a StoredObject, StoreError> {
        if state.quarantined.contains(id) {
            return Err(StoreError::Quarantined);
        }
        if let Some(object) = state.objects.get(id) {
            return Ok(object);
        }
        if state.promised.contains(id) {
            return Err(StoreError::Promised);
        }
        Err(StoreError::NotPresent)
    }

    fn check_expected(state: &MemorySnapshot, update: &crate::RefUpdate) -> Result<(), StoreError> {
        let actual = state.references.get(&update.key);
        let matches = match &update.expected {
            ExpectedRef::Absent => actual.is_none(),
            ExpectedRef::Exact(expected) => actual == Some(expected),
        };
        matches.then_some(()).ok_or(StoreError::ReferenceConflict)
    }

    fn next_state(
        state: &MemorySnapshot,
        publication: &Publication,
        update: &crate::RefUpdate,
    ) -> Result<ReferenceState, StoreError> {
        let generation = state.references.get(&update.key).map_or(Ok(0), |old| {
            old.generation
                .checked_add(1)
                .ok_or(StoreError::GenerationOverflow)
        })?;
        Ok(ReferenceState {
            target: update.target.clone(),
            generation,
            operation: publication.operation.clone(),
        })
    }

    fn stage(
        publication: &Publication,
        state: &MemorySnapshot,
    ) -> Result<MemorySnapshot, StoreError> {
        let mut staged = MemorySnapshot {
            objects: state.objects.clone(),
            promised: state.promised.clone(),
            quarantined: state.quarantined.clone(),
            references: state.references.clone(),
            revision: state.revision,
        };
        for candidate in &publication.objects {
            let decoded = Self::decode(&candidate.id, &candidate.bytes)?;
            if let Some(existing) = staged.objects.get(&candidate.id) {
                if existing.bytes() != decoded.bytes() {
                    return Err(StoreError::InvalidObject(
                        rgit_objects::DecodeObjectError::Digest,
                    ));
                }
            } else {
                staged.objects.insert(candidate.id.clone(), decoded);
            }
            staged.promised.remove(&candidate.id);
        }

        let operation = Self::visible_object(&staged, &publication.operation)?;
        if operation.kind() != ObjectKind::Operation {
            return Err(StoreError::ReferenceKind);
        }

        let mut keys = BTreeSet::new();
        for update in &publication.updates {
            if !keys.insert(update.key.clone()) {
                return Err(StoreError::DuplicateReferenceUpdate);
            }
            Self::check_expected(state, update)?;
            let target = Self::visible_object(&staged, &update.target)?;
            if target.kind() != update.key.expected_kind() {
                return Err(StoreError::ReferenceKind);
            }
            if !reference_identity_matches(&update.key, target) {
                return Err(StoreError::ReferenceKind);
            }
        }

        Self::validate_operation(publication, operation, &staged)?;

        let mut roots = vec![ReferenceEdge {
            role: ReferenceRole::OperationAfter,
            expected_kind: Some(ObjectKind::Operation),
            id: publication.operation.clone(),
        }];
        roots.extend(publication.updates.iter().map(|update| ReferenceEdge {
            role: ReferenceRole::OperationAfter,
            expected_kind: Some(update.key.expected_kind()),
            id: update.target.clone(),
        }));
        let closure = Self::closure_from(&staged, &roots)?;
        Self::validate_affected_graphs(&closure)?;

        for update in &publication.updates {
            staged.references.insert(
                update.key.clone(),
                Self::next_state(state, publication, update)?,
            );
        }
        Ok(staged)
    }

    fn validate_operation(
        publication: &Publication,
        operation: &StoredObject,
        state: &MemorySnapshot,
    ) -> Result<(), StoreError> {
        let operation_map =
            value_map(operation.object().decoded().value()).ok_or(StoreError::OperationMismatch)?;
        let actions =
            value_array(value_field(operation_map, 8).ok_or(StoreError::OperationMismatch)?)
                .ok_or(StoreError::OperationMismatch)?;
        let mut consumed = BTreeSet::new();

        for update in &publication.updates {
            match &update.key {
                ReferenceKey::OperationHead => {
                    if update.target != publication.operation {
                        return Err(StoreError::OperationMismatch);
                    }
                    let parents = operation
                        .references()
                        .iter()
                        .filter(|edge| edge.role == ReferenceRole::OperationParent)
                        .map(|edge| &edge.id)
                        .collect::<BTreeSet<_>>();
                    match &update.expected {
                        ExpectedRef::Absent if !parents.is_empty() => {
                            return Err(StoreError::OperationMismatch);
                        }
                        ExpectedRef::Exact(state) if !parents.contains(&state.target) => {
                            return Err(StoreError::OperationMismatch);
                        }
                        _ => {}
                    }
                }
                ReferenceKey::Line(line_id) => {
                    let line_state = Self::visible_object(state, &update.target)?;
                    let line_state = value_map(line_state.object().decoded().value())
                        .ok_or(StoreError::OperationMismatch)?;
                    if value_bytes(value_field(line_state, 3).ok_or(StoreError::OperationMismatch)?)
                        != Some(line_id.as_bytes())
                    {
                        return Err(StoreError::OperationMismatch);
                    }
                    if object_id_field(line_state, 12).as_ref() != Some(&publication.operation) {
                        return Err(StoreError::OperationMismatch);
                    }
                    validate_line_predecessor(line_state, &update.expected)?;
                    let index = actions
                        .iter()
                        .enumerate()
                        .find_map(|(index, action)| {
                            (!consumed.contains(&index) && line_action_matches(action, line_state))
                                .then_some(index)
                        })
                        .ok_or(StoreError::OperationMismatch)?;
                    consumed.insert(index);
                }
                _ => {
                    let index = actions
                        .iter()
                        .enumerate()
                        .find_map(|(index, action)| {
                            (!consumed.contains(&index) && transition_matches(action, update))
                                .then_some(index)
                        })
                        .ok_or(StoreError::OperationMismatch)?;
                    consumed.insert(index);
                }
            }
        }
        if consumed.len() != actions.len() {
            return Err(StoreError::OperationMismatch);
        }
        Ok(())
    }

    fn closure_from(
        state: &MemorySnapshot,
        roots: &[ReferenceEdge],
    ) -> Result<Closure, StoreError> {
        let mut objects = BTreeMap::new();
        let mut pending = roots.to_vec();
        while let Some(edge) = pending.pop() {
            let object = match Self::visible_object(state, &edge.id) {
                Ok(object) => object,
                Err(StoreError::NotPresent) => return Err(ClosureError::Missing.into()),
                Err(StoreError::Promised) => return Err(ClosureError::Promised.into()),
                Err(StoreError::Quarantined) => return Err(ClosureError::Quarantined.into()),
                Err(error) => return Err(error),
            };
            if let Some(expected) = edge.expected_kind {
                if object.kind() != expected {
                    return Err(ClosureError::WrongKind {
                        expected,
                        actual: object.kind(),
                    }
                    .into());
                }
            }
            if objects.insert(edge.id, object.clone()).is_none() {
                pending.extend_from_slice(object.references());
            }
        }
        Ok(Closure::new(objects))
    }

    fn validate_affected_graphs(closure: &Closure) -> Result<(), StoreError> {
        for kind in [ObjectKind::Snapshot, ObjectKind::Operation] {
            Self::graph(closure.objects(), kind)?;
        }
        Ok(())
    }

    fn graph(
        objects: &BTreeMap<ObjectId, StoredObject>,
        kind: ObjectKind,
    ) -> Result<GraphIndex<ObjectId>, StoreError> {
        let parent_role = match kind {
            ObjectKind::Snapshot => ReferenceRole::SnapshotParent,
            ObjectKind::Operation => ReferenceRole::OperationParent,
            _ => return Err(StoreError::InvalidGraph),
        };
        let nodes = objects
            .values()
            .filter(|object| object.kind() == kind)
            .map(|object| {
                GraphNode::new(
                    object.id().clone(),
                    object
                        .references()
                        .iter()
                        .filter(|edge| edge.role == parent_role)
                        .map(|edge| edge.id.clone())
                        .collect(),
                )
            });
        GraphIndex::build(nodes).map_err(|_| StoreError::InvalidGraph)
    }

    fn ancestry_closure(
        state: &MemorySnapshot,
        roots: &[ObjectId],
        kind: ObjectKind,
    ) -> Result<BTreeMap<ObjectId, StoredObject>, StoreError> {
        let parent_role = match kind {
            ObjectKind::Snapshot => ReferenceRole::SnapshotParent,
            ObjectKind::Operation => ReferenceRole::OperationParent,
            _ => return Err(StoreError::InvalidGraph),
        };
        let mut result = BTreeMap::new();
        let mut pending = roots.to_vec();
        while let Some(id) = pending.pop() {
            let object = Self::visible_object(state, &id)?;
            if object.kind() != kind {
                return Err(StoreError::InvalidGraph);
            }
            if result.insert(id, object.clone()).is_none() {
                pending.extend(
                    object
                        .references()
                        .iter()
                        .filter(|edge| edge.role == parent_role)
                        .map(|edge| edge.id.clone()),
                );
            }
        }
        Ok(result)
    }

    #[allow(dead_code)]
    pub(crate) fn raw_ids(&self) -> Vec<ObjectId> {
        self.lock().objects.keys().cloned().collect()
    }
}

impl Store for MemoryStore {
    fn put(&self, id: ObjectId, bytes: Vec<u8>) -> Result<PutOutcome, StoreError> {
        let decoded = Self::decode(&id, &bytes)?;
        let mut state = self.lock();
        if let Some(existing) = state.objects.get(&id) {
            return if existing.bytes() == decoded.bytes() {
                Ok(PutOutcome::AlreadyPresent)
            } else {
                Err(StoreError::InvalidObject(
                    rgit_objects::DecodeObjectError::Digest,
                ))
            };
        }
        let next_revision = state
            .revision
            .checked_add(1)
            .ok_or(StoreError::RevisionOverflow)?;
        state.objects.insert(id.clone(), decoded);
        state.promised.remove(&id);
        state.revision = next_revision;
        Ok(PutOutcome::New)
    }

    fn get(&self, id: &ObjectId) -> Result<StoredObject, StoreError> {
        Ok(Self::visible_object(&self.lock(), id)?.clone())
    }

    fn presence(&self, id: &ObjectId) -> Option<ObjectPresence> {
        let state = self.lock();
        if state.quarantined.contains(id) {
            Some(ObjectPresence::Quarantined)
        } else if state.objects.contains_key(id) {
            Some(ObjectPresence::Present)
        } else if state.promised.contains(id) {
            Some(ObjectPresence::Promised)
        } else {
            None
        }
    }

    fn mark_promised(&self, id: ObjectId) -> Result<(), StoreError> {
        let mut state = self.lock();
        if !state.objects.contains_key(&id) && !state.promised.contains(&id) {
            let next_revision = state
                .revision
                .checked_add(1)
                .ok_or(StoreError::RevisionOverflow)?;
            state.promised.insert(id);
            state.revision = next_revision;
        }
        Ok(())
    }

    fn quarantine(&self, id: &ObjectId) -> Result<(), StoreError> {
        let mut state = self.lock();
        if !state.objects.contains_key(id) && !state.promised.contains(id) {
            return Err(StoreError::NotPresent);
        }
        if !state.quarantined.contains(id) {
            let next_revision = state
                .revision
                .checked_add(1)
                .ok_or(StoreError::RevisionOverflow)?;
            state.quarantined.insert(id.clone());
            state.revision = next_revision;
        }
        Ok(())
    }

    fn reference(&self, key: &ReferenceKey) -> Option<ReferenceState> {
        self.lock().references.get(key).cloned()
    }

    fn compare_and_swap(
        &self,
        key: ReferenceKey,
        expected: ExpectedRef,
        target: ObjectId,
        operation: ObjectId,
        validator: &dyn PublicationValidator,
    ) -> Result<ReferenceState, StoreError> {
        let publication = Publication {
            objects: Vec::new(),
            updates: vec![crate::RefUpdate {
                key,
                expected,
                target,
            }],
            operation,
        };
        self.publish(publication, validator)?
            .into_iter()
            .next()
            .ok_or(StoreError::ReferenceConflict)
    }

    fn publish(
        &self,
        publication: Publication,
        validator: &dyn PublicationValidator,
    ) -> Result<Vec<ReferenceState>, StoreError> {
        let (mut staged, base_revision, next_revision) = {
            let state = self.lock();
            let next_revision = state
                .revision
                .checked_add(1)
                .ok_or(StoreError::RevisionOverflow)?;
            (
                Self::stage(&publication, &state)?,
                state.revision,
                next_revision,
            )
        };
        validator.validate(&PublicationCandidate {
            publication: &publication,
            objects: &staged.objects,
            references: &staged.references,
        })?;
        let results = publication
            .updates
            .iter()
            .filter_map(|update| staged.references.get(&update.key).cloned())
            .collect();
        let mut state = self.lock();
        if state.revision != base_revision {
            return Err(StoreError::ReferenceConflict);
        }
        staged.revision = next_revision;
        *state = staged;
        Ok(results)
    }

    fn closure(&self, roots: &[ReferenceEdge]) -> Result<Closure, StoreError> {
        Self::closure_from(&self.lock(), roots)
    }

    fn generation(&self, id: &ObjectId) -> Result<u64, StoreError> {
        let state = self.lock();
        let object = Self::visible_object(&state, id)?;
        let objects = Self::ancestry_closure(&state, std::slice::from_ref(id), object.kind())?;
        Self::graph(&objects, object.kind())?
            .generation(id)
            .map_err(|_| StoreError::InvalidGraph)
    }

    fn is_reachable(&self, ancestor: &ObjectId, descendant: &ObjectId) -> Result<bool, StoreError> {
        let state = self.lock();
        let ancestor_object = Self::visible_object(&state, ancestor)?;
        let descendant_object = Self::visible_object(&state, descendant)?;
        if ancestor_object.kind() != descendant_object.kind() {
            return Err(StoreError::InvalidGraph);
        }
        let objects = Self::ancestry_closure(
            &state,
            std::slice::from_ref(descendant),
            ancestor_object.kind(),
        )?;
        if !objects.contains_key(ancestor) {
            return Ok(false);
        }
        Self::graph(&objects, ancestor_object.kind())?
            .is_reachable(ancestor, descendant)
            .map_err(|_| StoreError::InvalidGraph)
    }
}

fn value_map(value: &Value) -> Option<&[(u64, Value)]> {
    if let Value::Map(map) = value {
        Some(map)
    } else {
        None
    }
}
fn value_array(value: &Value) -> Option<&[Value]> {
    if let Value::Array(array) = value {
        Some(array)
    } else {
        None
    }
}
fn value_field(map: &[(u64, Value)], key: u64) -> Option<&Value> {
    map.iter()
        .find(|(actual, _)| *actual == key)
        .map(|(_, value)| value)
}
fn value_bytes(value: &Value) -> Option<&[u8]> {
    if let Value::Bytes(bytes) = value {
        Some(bytes)
    } else {
        None
    }
}
fn value_unsigned(value: &Value) -> Option<u64> {
    if let Value::Unsigned(number) = value {
        Some(*number)
    } else {
        None
    }
}
fn object_id(value: &Value) -> Option<ObjectId> {
    ObjectId::from_bytes(value_bytes(value)?).ok()
}
fn object_id_field(map: &[(u64, Value)], key: u64) -> Option<ObjectId> {
    object_id(value_field(map, key)?)
}
fn typed_ref(value: &Value) -> Option<(ObjectKind, ObjectId)> {
    let map = value_map(value)?;
    let kind = ObjectKind::try_from(value_unsigned(value_field(map, 0)?)?).ok()?;
    Some((kind, object_id_field(map, 1)?))
}
fn validate_line_predecessor(
    state: &[(u64, Value)],
    expected: &ExpectedRef,
) -> Result<(), StoreError> {
    let generation = value_unsigned(value_field(state, 6).ok_or(StoreError::OperationMismatch)?)
        .ok_or(StoreError::OperationMismatch)?;
    let previous = value_field(state, 7).and_then(object_id);
    match expected {
        ExpectedRef::Absent if generation == 0 && previous.is_none() => Ok(()),
        ExpectedRef::Exact(old)
            if generation
                == old
                    .generation
                    .checked_add(1)
                    .ok_or(StoreError::GenerationOverflow)?
                && previous.as_ref() == Some(&old.target) =>
        {
            Ok(())
        }
        _ => Err(StoreError::OperationMismatch),
    }
}
fn line_action_matches(action: &Value, state: &[(u64, Value)]) -> bool {
    let Some(action) = value_map(action) else {
        return false;
    };
    if value_field(action, 0).and_then(value_unsigned) != Some(1) {
        return false;
    }
    let Some(declaration) = value_field(action, 3).and_then(value_map) else {
        return false;
    };
    [
        (0, 2),
        (1, 3),
        (2, 4),
        (3, 5),
        (4, 6),
        (6, 8),
        (7, 9),
        (8, 10),
        (9, 11),
    ]
    .iter()
    .all(|(declaration_key, state_key)| {
        value_field(declaration, *declaration_key) == value_field(state, *state_key)
    }) && value_field(declaration, 5) == value_field(state, 7)
}
fn transition_matches(action: &Value, update: &crate::RefUpdate) -> bool {
    let Some(action) = value_map(action) else {
        return false;
    };
    if value_field(action, 0).and_then(value_unsigned) != Some(0) {
        return false;
    }
    let before = value_field(action, 1).and_then(typed_ref);
    let after = value_field(action, 2).and_then(typed_ref);
    let expected_before = match &update.expected {
        ExpectedRef::Absent => None,
        ExpectedRef::Exact(state) => Some((update.key.expected_kind(), state.target.clone())),
    };
    before == expected_before && after == Some((update.key.expected_kind(), update.target.clone()))
}

pub(crate) fn reference_identity_matches(key: &ReferenceKey, target: &StoredObject) -> bool {
    let Some(map) = value_map(target.object().decoded().value()) else {
        return false;
    };
    match key {
        ReferenceKey::Line(id) | ReferenceKey::Release(id) => {
            value_field(map, 3).and_then(value_bytes) == Some(id.as_bytes())
        }
        ReferenceKey::Change(id) => {
            value_field(map, 3).and_then(value_bytes) == Some(id.as_bytes())
        }
        ReferenceKey::OperationHead | ReferenceKey::Marker(_) => true,
    }
}
