# Rethinking Git Implementation Checklist

Companion document: [implementation-plan.md](implementation-plan.md)
Status legend: `[ ]` not started · `[-]` in progress · `[x]` complete · `[!]` blocked

This checklist tracks implementation of the production-grade, permission-aware Git replacement. A milestone is complete only when every task and exit criterion in that milestone is checked.

## Project decisions

- [x] Approve `change + snapshot + operation` as the permanent core model.
- [x] Approve permissioned `view` as a storage and synchronization security boundary.
- [ ] Approve isolated overlays as the workspace-session model.
- [x] Approve signed audience projections as the release model.
- [x] Approve Git interoperability as an adapter rather than the native data model.
- [x] Record unresolved product or architecture decisions as ADRs.
- [ ] Assign owners for storage, graph, security, workspace, sync, Git compatibility, CLI, and reliability.
- [x] Establish milestone review and sign-off process.

## Milestone 0 — Baseline and specifications

### Continuous integration

- [x] Add Rust formatting checks.
- [x] Add Clippy with warnings treated according to the project lint policy.
- [ ] Run unit and integration tests in CI.
- [x] Add dependency vulnerability and license checks.
- [x] Add Linux CI.
- [x] Add macOS CI.
- [x] Add Windows CI for supported behavior.
- [x] Cache Cargo dependencies without hiding reproducibility problems.
- [ ] Publish test and benchmark artifacts from CI.

### Baseline behavior

- [x] Create golden tests for `rgit init`.
- [x] Create golden tests for change creation, listing, and inspection.
- [x] Create golden tests for snapshot creation, listing, and inspection.
- [x] Create golden tests for status and workspace information.
- [x] Create golden tests for workspace, snapshot, and line diffs.
- [x] Create golden tests for actor and path-policy management.
- [x] Create golden tests for permission-filtered views.
- [x] Create golden tests for line integration.
- [x] Create golden tests for merge preview.
- [x] Create golden tests for conflict creation and inspection.
- [x] Create golden tests for redacted operation history.
- [x] Add fixtures for public-only repositories.
- [x] Add fixtures for mixed public and restricted paths.
- [x] Add a security-fix fixture.
- [x] Add a production-configuration/secret-reference fixture.
- [x] Add concurrent-change and merge-conflict fixtures.

### Baseline performance

- [x] Select benchmark hardware profiles.
- [x] Select small, medium, Linux-scale, and monorepo fixtures.
- [x] Benchmark repository initialization.
- [x] Benchmark working-tree scanning.
- [x] Benchmark snapshot creation.
- [x] Benchmark warm and cold status.
- [x] Benchmark file and snapshot diff.
- [x] Benchmark three-way merge planning.
- [x] Save results in a versioned benchmark report.
- [x] Define initial latency, memory, and disk budgets.

### Specifications and ADRs

- [x] Write the initial object-model specification.
- [x] Write the threat-model skeleton.
- [x] Inventory trusted components and trust boundaries.
- [x] Inventory content and metadata leakage risks.
- [x] Document revocation and previously decrypted plaintext limitations.
- [x] Decide canonical object encoding.
- [x] Decide hash algorithm and algorithm-agility strategy.
- [x] Decide embedded transactional database.
- [x] Decide immutable pack/chunk layout direction.
- [x] Decide cryptographic libraries and supported primitives.
- [x] Decide async runtime.
- [x] Decide RPC framework and transport.
- [x] Decide minimum supported Rust version.
- [x] Decide supported operating systems and filesystems.

### Milestone 0 exit criteria

- [ ] All documented prototype flows run in CI.
- [x] Golden tests protect existing semantics.
- [x] Benchmark baselines are reproducible.
- [x] Threat-model and trust-boundary documents are reviewed.
- [x] Required ADRs are approved.
- [ ] Milestone 0 review is signed off.

## Milestone 1 — Cargo workspace and production object store

### Workspace refactor

- [x] Convert the repository to a Cargo workspace.
- [x] Create `rgit-objects`.
- [x] Create `rgit-store`.
- [x] Create `rgit-graph`.
- [ ] Create `rgit-operations`.
- [ ] Create `rgit-cli`.
- [ ] Create `rgit-testkit`.
- [ ] Enforce intended crate dependency direction.
- [ ] Move orchestration out of library code.
- [ ] Replace direct library printing with typed results.
- [x] Preserve current CLI behavior during the refactor.

### Canonical objects

- [x] Define versioned object IDs.
- [x] Define canonical blob encoding.
- [x] Define canonical path segments and normalization.
- [x] Define manifest entry types.
- [x] Define canonical manifest ordering.
- [x] Define snapshot schema.
- [x] Define change schema.
- [x] Define line schema and generation counter.
- [x] Define conflict schema.
- [x] Define operation schema.
- [x] Define policy-label reference schema.
- [x] Add schema-version fields.
- [x] Add canonical encoding test vectors.
- [ ] Add cross-platform decode tests.
- [x] Add debug JSON rendering for canonical objects.

### Object store

- [x] Implement an in-memory store for reference tests.
- [x] Implement immutable loose-object writes.
- [ ] Implement transactional metadata and indexes.
- [x] Write through temporary files.
- [x] Flush file content before publication.
- [x] Atomically rename published object data.
- [ ] Commit metadata only after referenced objects are durable.
- [x] Implement streaming object reads.
- [ ] Implement chunked large-object storage.
- [x] Deduplicate identical logical content.
- [ ] Implement reachability indexes.
- [ ] Implement graph generation numbers.
- [ ] Add repository format version and migration framework.

### Verification and recovery

- [x] Implement object digest verification.
- [ ] Implement reference integrity verification.
- [ ] Detect missing referenced objects.
- [x] Detect invalid or unsupported schema versions.
- [ ] Produce non-destructive repair reports.
- [ ] Add transaction failure-injection points.
- [ ] Test process termination between transaction phases.
- [ ] Test partial writes and corrupted indexes.
- [ ] Prove a line cannot point at a missing object after recovery.

### Milestone 1 exit criteria

- [ ] Existing prototype scenarios pass on the new store.
- [ ] Canonical test vectors are byte-stable across supported platforms.
- [ ] Identical logical objects deduplicate.
- [ ] Repository verification works without CLI in-memory state.
- [ ] Failure-injection tests demonstrate crash-safe reference updates.
- [ ] Milestone 1 review is signed off.

## Milestone 2 — Complete local version-control semantics

### Change lifecycle

- [ ] Implement `change start`.
- [ ] Implement change title and description editing.
- [ ] Implement change listing and filtering.
- [ ] Implement change abandon.
- [ ] Implement change restore.
- [ ] Implement change split.
- [ ] Implement change combine.
- [ ] Implement change retarget.
- [ ] Preserve stable change IDs through snapshot evolution.

### Markers and releases

- [ ] Implement typed marker object.
- [ ] Implement release marker type.
- [ ] Implement deployment marker type.
- [ ] Implement review marker type.
- [ ] Implement policy marker type.
- [ ] Implement bookmark marker type.
- [ ] Implement immutable local release object.
- [ ] Add marker and release inspection commands.

### Automatic snapshots

- [ ] Add filesystem watcher abstraction.
- [ ] Add Linux watcher backend.
- [ ] Add macOS watcher backend.
- [ ] Add Windows watcher backend.
- [ ] Reconcile watcher state at every command boundary.
- [ ] Handle missed, coalesced, and reordered filesystem events.
- [ ] Capture recoverable snapshots without manual commands.
- [ ] Avoid blocking normal editing during snapshot creation.
- [ ] Add snapshot retention and compaction rules.

### Filesystem semantics

- [ ] Implement ignore rules.
- [ ] Import common `.gitignore` behavior.
- [ ] Implement materialization rules.
- [ ] Track executable/file mode.
- [ ] Track symbolic links safely.
- [ ] Define empty-directory behavior.
- [ ] Add rename heuristics.
- [ ] Add copy heuristics.
- [ ] Add subproject-reference object support.
- [ ] Detect case-fold path collisions.
- [ ] Detect Unicode-normalization path collisions.
- [ ] Prevent path traversal.

### Operations and recovery

- [ ] Make every state-changing command an operation transaction.
- [ ] Implement operation parent relationships.
- [ ] Implement operation log inspection.
- [ ] Implement undo.
- [ ] Implement redo.
- [ ] Define non-reversible operation behavior.
- [ ] Recover interrupted commands.
- [ ] Prevent concurrent local processes from corrupting state.

### CLI usability

- [ ] Add line-level text patches.
- [ ] Add binary-file reporting.
- [ ] Add color and paging.
- [ ] Add stable JSON output.
- [ ] Add documented process exit codes.
- [ ] Add shell completion.
- [ ] Add `repo verify`.
- [ ] Add `repo compact`.
- [ ] Add `repo gc` with a recovery grace period.
- [ ] Make line updates compare-and-swap transactions.

### Milestone 2 exit criteria

- [ ] Ordinary solo development requires no Git commands.
- [ ] Normal edits are automatically recoverable.
- [ ] Destructive-looking local actions have operation-level recovery.
- [ ] Concurrent local CLI processes cannot corrupt the repository.
- [ ] Status and snapshot performance meet milestone budgets.
- [ ] Milestone 2 review is signed off.

## Milestone 3 — Merge, conflicts, and landing

### Merge engine

- [ ] Move three-snapshot merge planning into `rgit-graph`.
- [ ] Implement file-level three-way merge.
- [ ] Implement line-based text merge.
- [ ] Store conflict regions as objects.
- [ ] Handle add/add conflicts.
- [ ] Handle modify/modify conflicts.
- [ ] Handle delete/modify conflicts.
- [ ] Handle file/directory conflicts.
- [ ] Handle mode and symlink conflicts.
- [ ] Add binary merge-driver behavior.

### Structured merge drivers

- [ ] Define merge-driver interface.
- [ ] Add JSON merge driver.
- [ ] Add TOML merge driver.
- [ ] Add YAML merge driver or document why it is deferred.
- [ ] Add lockfile merge strategy.
- [ ] Add generated-file strategy.
- [ ] Allow repository policy to select merge drivers.
- [ ] Sandbox external merge drivers.

### Conflict workflow

- [ ] Persist base, target, and incoming conflict sides.
- [ ] Persist conflict policy labels.
- [ ] Assign conflicts to authorized actors.
- [ ] Save candidate resolutions.
- [ ] Record resolution provenance.
- [ ] Hide restricted conflict paths from unauthorized users.
- [ ] Hide restricted conflict sides from unauthorized users.
- [ ] Add conflict resolve command.
- [ ] Add conflict abandon/reset command.
- [ ] Add conservative reusable resolution memory.

### Landing

- [ ] Implement virtual integration against the current target head.
- [ ] Implement `rgit land <line>`.
- [ ] Preserve stable change identity during recomputation.
- [ ] Add expected-head generation checks.
- [ ] Recompute after a concurrent line advance.
- [ ] Advance the line atomically only after successful checks.
- [ ] Add configurable linear topology policy.
- [ ] Add merge-preserving topology policy.
- [ ] Add squash projection policy.
- [ ] Keep topology controls out of the default daily workflow.

### Merge correctness

- [ ] Add merge property tests.
- [ ] Add merge fuzzing.
- [ ] Verify identical changes converge.
- [ ] Verify unchanged input is not lost.
- [ ] Verify permission labels propagate to results.
- [ ] Verify conflict records include every contributing restriction.

### Milestone 3 exit criteria

- [ ] `rgit land main` advances atomically or returns durable conflicts.
- [ ] Concurrent line changes cannot be overwritten silently.
- [ ] Unauthorized identities cannot inspect restricted conflict details.
- [ ] Default integration never asks users to choose merge or rebase.
- [ ] Merge correctness and fuzz suites pass.
- [ ] Milestone 3 review is signed off.

## Milestone 4 — Identity, capabilities, and encryption

### Identity

- [ ] Create `rgit-crypto`.
- [ ] Create `rgit-policy`.
- [ ] Define user identities.
- [ ] Define service identities.
- [ ] Define agent identities.
- [ ] Define device identities.
- [ ] Implement identity key generation.
- [ ] Implement identity enrollment.
- [ ] Implement identity rotation.
- [ ] Implement device removal.
- [ ] Remove `--as` as an authentication mechanism.

### Capabilities and policy

- [ ] Define discover capability.
- [ ] Define read capability.
- [ ] Define materialize capability.
- [ ] Define derive capability.
- [ ] Define review capability.
- [ ] Define integrate capability.
- [ ] Define release capability.
- [ ] Define administer capability.
- [ ] Define audit capability.
- [ ] Implement scoped capability issuance.
- [ ] Implement capability expiry.
- [ ] Implement capability signature verification.
- [ ] Implement immutable policy versions.
- [ ] Separate object discovery from object reading.

### Object encryption

- [ ] Select and document authenticated-encryption construction.
- [ ] Encrypt blob content.
- [ ] Encrypt private manifests.
- [ ] Encrypt private change metadata.
- [ ] Encrypt private operation payloads.
- [ ] Encrypt private conflict data.
- [ ] Encrypt private indexes or redesign them to avoid leakage.
- [ ] Generate random data-encryption keys.
- [ ] Wrap keys for authorized policy/key epochs.
- [ ] Support multiple authorized recipients or policy groups.
- [ ] Authenticate object type and schema as associated data.
- [ ] Detect ciphertext substitution and rollback where in scope.

### Key lifecycle

- [ ] Define key epochs.
- [ ] Implement key rotation.
- [ ] Implement envelope rewrapping without logical-history rewrite.
- [ ] Implement organization recovery/escrow option.
- [ ] Define lost-key behavior.
- [ ] Define revoked-device behavior.
- [ ] Prevent revoked devices from receiving new keys.
- [ ] Document inability to erase previously decrypted plaintext.

### Policy-safe derivation

- [ ] Label diff outputs.
- [ ] Label merge outputs.
- [ ] Label conflict outputs.
- [ ] Label search-index entries.
- [ ] Label cache entries.
- [ ] Label build artifacts.
- [ ] Label exported projections.
- [ ] Label AI/agent context bundles.
- [ ] Implement conservative policy combination.
- [ ] Add explicit declassification operations.
- [ ] Require configured approval/signatures for declassification.

### Leakage prevention

- [ ] Remove secrets from logs.
- [ ] Remove secrets from error messages.
- [ ] Remove secrets from crash reports.
- [ ] Remove secrets from shell completion.
- [ ] Remove paths and sensitive metadata from telemetry.
- [ ] Test unauthorized object enumeration.
- [ ] Test metadata redaction behavior.
- [ ] Test cache separation across policy labels.

### Milestone 4 exit criteria

- [ ] Copied storage does not reveal protected content covered by the threat model.
- [ ] Unauthorized identities cannot enumerate restricted object IDs through supported APIs.
- [ ] Revoked devices cannot retrieve new decryption keys.
- [ ] Every object-producing path has tested policy propagation.
- [ ] Cryptographic construction and key lifecycle are documented.
- [ ] External cryptographic design review is complete.
- [ ] All critical review findings are resolved.
- [ ] Marketing and documentation use only reviewed security claims.
- [ ] Milestone 4 review is signed off.

## Milestone 5 — Permissioned workspace sessions

### Session model

- [ ] Create `rgit-workspace`.
- [ ] Define workspace-session ID.
- [ ] Bind identity to a session.
- [ ] Bind change to a session.
- [ ] Bind permissioned view to a session.
- [ ] Bind environment profile to a session.
- [ ] Bind writable overlay to a session.
- [ ] Implement session create.
- [ ] Implement session enter.
- [ ] Implement session list and inspect.
- [ ] Implement session pause and resume.
- [ ] Implement session discard.
- [ ] Implement session recovery.

### Materialization backends

- [ ] Define portable materialization interface.
- [ ] Implement Linux overlayfs backend where available.
- [ ] Implement Linux FUSE fallback.
- [ ] Implement macOS provider/FUSE or managed-directory backend.
- [ ] Implement Windows ProjFS or managed-directory backend.
- [ ] Implement safe read-only base sharing.
- [ ] Store only session-specific writable deltas.
- [ ] Handle backend crash and remount.
- [ ] Prevent symlink escape from a materialized view.
- [ ] Prevent restricted files from entering unauthorized overlays.

### Session-aware tools

- [ ] Implement session-aware command broker.
- [ ] Route shells to the correct session.
- [ ] Route editors to the correct session.
- [ ] Route agents to the correct session.
- [ ] Route tests and builds to the correct session.
- [ ] Issue scoped session capability tokens.
- [ ] Capture session changes automatically.
- [ ] Ensure `main` is never exclusively checked out.

### Shared caches

- [ ] Define immutable dependency-download cache.
- [ ] Define Rust artifact cache key.
- [ ] Define JavaScript package cache key.
- [ ] Include lockfile in cache keys.
- [ ] Include toolchain/compiler in cache keys.
- [ ] Include target and features in cache keys.
- [ ] Include relevant environment and policy labels in cache keys.
- [ ] Prevent concurrent writable-cache corruption.
- [ ] Prevent cache poisoning between policy domains.
- [ ] Measure cold and warm session startup.

### Environment and secrets

- [ ] Define versioned environment-profile schema.
- [ ] Track non-secret environment values.
- [ ] Track secret references.
- [ ] Validate environment values against schemas.
- [ ] Resolve authorized secret references at session start or process launch.
- [ ] Prefer process environment or ephemeral mounts over durable files.
- [ ] Remove materialized secrets when sessions stop.
- [ ] Audit secret access without logging secret values.

### Milestone 5 exit criteria

- [ ] Two agents can edit different changes concurrently in one project.
- [ ] Agent sessions cannot overwrite each other’s writable source state.
- [ ] Sessions share safe dependency and build artifacts.
- [ ] Unauthorized sessions never receive restricted backing objects.
- [ ] Session crashes do not lose captured work.
- [ ] Warm session creation beats worktree plus dependency installation targets.
- [ ] Workspace isolation security review is complete.
- [ ] Milestone 5 review is signed off.

## Milestone 6 — Native sync and remote server

### Protocol

- [ ] Create `rgit-sync`.
- [ ] Create `rgit-server`.
- [ ] Specify protocol version negotiation.
- [ ] Specify repository capability negotiation.
- [ ] Authenticate clients before discovery.
- [ ] Implement authorization-before-discovery negotiation.
- [ ] Implement streaming upload.
- [ ] Implement streaming download.
- [ ] Implement resumable transfers.
- [ ] Verify transferred object integrity.
- [ ] Make operation publication idempotent.
- [ ] Add partial-view synchronization.
- [ ] Add offline operation queue.
- [ ] Add reconnect reconciliation.

### Remote state

- [ ] Implement atomic line compare-and-swap.
- [ ] Implement generation-conflict responses.
- [ ] Implement retry/recompute workflow.
- [ ] Separate encrypted object storage from searchable metadata.
- [ ] Replicate signed audit operations.
- [ ] Enforce retention rules.
- [ ] Protect in-flight and newly uploaded objects from premature GC.

### Identity service integration

- [ ] Implement user authentication.
- [ ] Implement organization membership.
- [ ] Implement service accounts.
- [ ] Implement agent identities.
- [ ] Implement device enrollment.
- [ ] Implement capability issuance and renewal.
- [ ] Implement revocation propagation.
- [ ] Add enterprise identity integration interface.

### Operations

- [ ] Add quotas.
- [ ] Add rate limits.
- [ ] Add abuse protections.
- [ ] Add backup.
- [ ] Add restore.
- [ ] Add storage verification.
- [ ] Add disaster-recovery runbook.
- [ ] Run disaster-recovery drills.
- [ ] Package a single-node self-hosted deployment.
- [ ] Document upgrade and rollback procedures.

### End-to-end security-fix slice

- [ ] Create restricted security change on client A.
- [ ] Synchronize only authorized objects to client B.
- [ ] Confirm unauthorized client cannot discover private change metadata.
- [ ] Run restricted CI with authorized capability.
- [ ] Land into an embargoed line.
- [ ] Publish a signed public projection.
- [ ] Verify public projection contains no restricted objects.
- [ ] Reconstruct complete provenance as an authorized auditor.

### Milestone 6 exit criteria

- [ ] Different actors synchronize different authorized views of one line.
- [ ] Unauthorized clients never receive a decryptable restricted object/key combination.
- [ ] Interrupted transfers resume correctly.
- [ ] Concurrent landing attempts respect line generations.
- [ ] Backups restore objects, lines, policies, keys, and operations.
- [ ] End-to-end security-fix slice passes.
- [ ] Sync authorization and key distribution review is complete.
- [ ] Milestone 6 review is signed off.

## Milestone 7 — Review, CI, and signed releases

### Review

- [ ] Implement published-change state.
- [ ] Implement reviewer assignment.
- [ ] Implement permission-aware comments.
- [ ] Implement approvals.
- [ ] Implement approval dismissal rules.
- [ ] Implement path ownership.
- [ ] Implement policy-aware review diffs.
- [ ] Hide restricted review metadata from unauthorized users.

### CI and landing queue

- [ ] Model CI results as signed attestations.
- [ ] Bind attestations to exact snapshots.
- [ ] Bind attestations to environment/toolchain inputs.
- [ ] Prevent attestation replay against different inputs.
- [ ] Define CI runner capabilities.
- [ ] Prevent restricted jobs from running on unauthorized runners.
- [ ] Implement landing queue.
- [ ] Recompute queued integrations after line advances.
- [ ] Require configured checks before line advancement.

### Releases

- [ ] Implement audience-specific projection.
- [ ] Implement signed release manifest.
- [ ] Attach source line and exact source snapshot.
- [ ] Attach build provenance.
- [ ] Attach artifact references.
- [ ] Add reproducibility metadata.
- [ ] Add embargo scheduling.
- [ ] Add coordinated multi-audience publication.
- [ ] Add explicit release declassification decision.
- [ ] Verify release contains only approved audience objects.
- [ ] Support independent release signature verification.

### CI secrets

- [ ] Resolve secret references only on authorized runners.
- [ ] Use short-lived secret capabilities.
- [ ] Prevent secret values from entering snapshots or attestations.
- [ ] Audit secret access without value leakage.
- [ ] Rotate secrets after relevant access changes.

### Milestone 7 exit criteria

- [ ] Lines enforce configured ownership, approval, test, and signature rules.
- [ ] CI attestations are input-bound and verifiable.
- [ ] Embargoed changes remain unavailable before explicit release.
- [ ] Public releases contain no private objects or metadata.
- [ ] Authorized auditors can reconstruct private provenance.
- [ ] Release manifests and signatures verify independently.
- [ ] Milestone 7 review is signed off.

## Milestone 8 — Git interoperability and migration

### Git import

- [ ] Create `rgit-gitbridge`.
- [ ] Import Git blobs.
- [ ] Import Git trees.
- [ ] Import Git commits.
- [ ] Import branches and refs.
- [ ] Import annotated and lightweight tags.
- [ ] Import authors, committers, and timestamps.
- [ ] Import supported signatures.
- [ ] Import supported attributes and ignore rules.
- [ ] Map Git objects to native IDs deterministically.
- [ ] Map Git branches according to explicit policy.
- [ ] Infer changes where possible.
- [ ] Report that inferred changes are not lossless Git facts.
- [ ] Detect unsupported or unusual Git constructs.

### Git export

- [ ] Define audience-projection export rules.
- [ ] Export public blobs, trees, and commits.
- [ ] Export branches/lines.
- [ ] Export release markers as tags.
- [ ] Export authorship and supported signatures.
- [ ] Refuse export when policy would leak restricted content.
- [ ] Prevent leakage through commit messages.
- [ ] Prevent leakage through ref names.
- [ ] Prevent leakage through mapping metadata.
- [ ] Prevent leakage through packfile reachability.

### Bridging and mirroring

- [ ] Implement Git remote helper or gateway.
- [ ] Support authorized fetch projections.
- [ ] Support guarded push mapping.
- [ ] Implement one-way mirror mode.
- [ ] Implement two-way mirror mode.
- [ ] Define force-update behavior.
- [ ] Define mirror divergence and conflict behavior.
- [ ] Validate mirrors continuously.

### Compatibility matrix

- [ ] Document Git LFS support.
- [ ] Document submodule support.
- [ ] Document shallow-clone behavior.
- [ ] Document partial-clone behavior.
- [ ] Document signature support.
- [ ] Document hooks and attributes.
- [ ] Document notes and replace refs.
- [ ] Document unusual object and ref forms.
- [ ] Classify each feature as lossless, projected, lossy, unsupported, or unsafe.

### Migration validation

- [ ] Compare imported file trees.
- [ ] Compare ref targets.
- [ ] Compare authorship and timestamps.
- [ ] Compare release tags/markers.
- [ ] Produce deterministic mapping report.
- [ ] Report every lossy conversion before cutover.
- [ ] Test rollback to the Git source of truth.

### Milestone 8 exit criteria

- [ ] Representative Git repositories import with matching public file state.
- [ ] Standard Git clients accept exported projections.
- [ ] Default export contains no restricted objects or metadata.
- [ ] Native users can coexist with an existing mirrored Git remote.
- [ ] Migration reports identify every lossy or unsupported construct.
- [ ] Git projection security review is complete.
- [ ] Milestone 8 review is signed off.

## Milestone 9 — Performance, reliability, and ecosystem

### Performance

- [ ] Profile status.
- [ ] Profile snapshotting.
- [ ] Profile graph traversal.
- [ ] Profile diff and merge.
- [ ] Profile policy evaluation.
- [ ] Profile encryption and decryption.
- [ ] Profile pack/chunk operations.
- [ ] Profile synchronization.
- [ ] Add graph indexes where measurements justify them.
- [ ] Add path/Bloom filters where measurements justify them.
- [ ] Add compression/delta strategy where measurements justify it.
- [ ] Add background compaction.
- [ ] Test millions of objects.
- [ ] Test deep histories.
- [ ] Test large binary files.
- [ ] Test high-latency and lossy networks.

### Reliability

- [ ] Add long-running concurrency tests.
- [ ] Add disk-full tests.
- [ ] Add slow-disk tests.
- [ ] Add process-crash campaigns.
- [ ] Add corrupted-object campaigns.
- [ ] Add clock-skew tests.
- [ ] Add network-partition tests.
- [ ] Add key-service outage tests.
- [ ] Add watcher-event loss tests.
- [ ] Define and measure recovery objectives.

### Interfaces and ecosystem

- [ ] Stabilize machine-readable CLI schemas.
- [ ] Version CLI output schemas.
- [ ] Define editor integration protocol.
- [ ] Add editor change views.
- [ ] Add editor diff views.
- [ ] Add editor conflict views.
- [ ] Add editor session views.
- [ ] Add editor policy views.
- [ ] Add editor operation/recovery views.
- [ ] Define agent SDK/API.
- [ ] Add protocol bindings only after protocol stabilization.
- [ ] Add version negotiation to integrations.

### Telemetry and privacy

- [ ] Define opt-in telemetry policy.
- [ ] Review telemetry with security/privacy owners.
- [ ] Collect latency and failure categories only.
- [ ] Exclude repository paths.
- [ ] Exclude content and object plaintext.
- [ ] Exclude sensitive identity and policy metadata.
- [ ] Add telemetry disable and inspection controls.

### Milestone 9 exit criteria

- [ ] Performance budgets pass on selected repository fixtures.
- [ ] Crash and fault campaigns meet recovery objectives.
- [ ] Public-repository storage and memory are competitive with Git targets.
- [ ] Encryption and selective-view overhead is measured and documented.
- [ ] CLI and integration APIs have explicit compatibility guarantees.
- [ ] Milestone 9 review is signed off.

## Milestone 10 — Audit, beta, and 1.0

### Format and release freeze

- [ ] Freeze 1.0 logical object schemas.
- [ ] Freeze 1.0 canonical encoding.
- [ ] Freeze 1.0 repository format.
- [ ] Freeze 1.0 sync protocol.
- [ ] Freeze 1.0 CLI compatibility guarantees.
- [ ] Complete migration from every supported pre-1.0 format.
- [ ] Test upgrade rollback where supported.

### Independent audits

- [ ] Complete cryptographic audit.
- [ ] Complete authorization and policy audit.
- [ ] Complete client storage audit.
- [ ] Complete server audit.
- [ ] Complete sync protocol audit.
- [ ] Complete workspace isolation audit.
- [ ] Complete Git export/projection audit.
- [ ] Resolve all critical findings.
- [ ] Resolve all high findings.
- [ ] Document accepted lower-severity risks.

### Beta

- [ ] Select design partners.
- [ ] Run private beta on non-critical repositories.
- [ ] Measure onboarding time.
- [ ] Measure daily workflow success.
- [ ] Run review and release cycles.
- [ ] Run recovery drills.
- [ ] Run key rotation and revocation drills.
- [ ] Run backup and restore drills.
- [ ] Run full Git export/exit drill.
- [ ] Resolve beta-blocking issues.
- [ ] Run public beta.
- [ ] Confirm design partners can operate without Git as source of truth.

### Operational readiness

- [ ] Publish operator documentation.
- [ ] Publish backup and restore documentation.
- [ ] Publish repository verification and repair documentation.
- [ ] Publish migration and export documentation.
- [ ] Publish incident-response runbooks.
- [ ] Establish vulnerability-reporting channel.
- [ ] Establish coordinated-disclosure process.
- [ ] Establish key-compromise response.
- [ ] Establish patch-release process.
- [ ] Define support and upgrade windows.
- [ ] Define backward compatibility policy.

### Supply chain

- [ ] Produce reproducible client builds.
- [ ] Produce reproducible server builds.
- [ ] Sign releases.
- [ ] Publish checksums.
- [ ] Publish an SBOM.
- [ ] Pin and audit release dependencies.
- [ ] Protect release signing keys.
- [ ] Verify release provenance independently.

### Milestone 10 exit criteria

- [ ] No unresolved critical or high audit findings.
- [ ] Production-like restore succeeds.
- [ ] Full Git export succeeds.
- [ ] Supported upgrades succeed and recovery paths are documented.
- [ ] Design partners complete development and release cycles without Git as source of truth.
- [ ] Public security claims exactly match the reviewed threat model.
- [ ] 1.0 release readiness review is signed off.

## Cross-cutting test checklist

### Unit and integration tests

- [ ] Maintain canonical encoding tests.
- [ ] Maintain hash-stability tests.
- [ ] Maintain policy and capability tests.
- [ ] Maintain key envelope and rotation tests.
- [ ] Maintain graph and reachability tests.
- [ ] Maintain diff and merge tests.
- [ ] Maintain operation recovery tests.
- [ ] Maintain path/filesystem edge-case tests.
- [ ] Maintain end-to-end public workflow tests.
- [ ] Maintain end-to-end restricted workflow tests.

### Property tests

- [ ] Object encoding round trips.
- [ ] Store transactions match the reference model.
- [ ] Merge invariants hold.
- [ ] Policy derivation never weakens restrictions implicitly.
- [ ] Sync converges after reorder, duplication, and interruption.
- [ ] Garbage collection retains every reachable/protected object.

### Fuzzing

- [ ] Fuzz object decoders.
- [ ] Fuzz protocol decoders.
- [ ] Fuzz pack and chunk readers.
- [ ] Fuzz path and manifest parsers.
- [ ] Fuzz Git import boundaries.
- [ ] Fuzz merge drivers.
- [ ] Fuzz policy and capability tokens.
- [ ] Run fuzzing continuously in CI or scheduled infrastructure.

### Adversarial tests

- [ ] Attempt unauthorized object enumeration.
- [ ] Attempt ciphertext swapping.
- [ ] Attempt object rollback.
- [ ] Attempt malicious graph ingestion.
- [ ] Attempt decompression bombs.
- [ ] Attempt error-message leakage.
- [ ] Attempt search-index leakage.
- [ ] Attempt conflict-view leakage.
- [ ] Attempt telemetry leakage.
- [ ] Attempt stale-capability use.
- [ ] Attempt symlink/path traversal.
- [ ] Attempt cross-policy cache poisoning.

## Security governance checklist

- [ ] Assign security workstream owner.
- [ ] Keep threat model current.
- [ ] Keep trust-boundary diagrams current.
- [ ] Keep cryptographic construction document current.
- [ ] Keep metadata-leakage inventory current.
- [ ] Keep authorization matrix current.
- [ ] Keep key lifecycle runbook current.
- [ ] Document derivation rule for every object-producing subsystem.
- [ ] Track audit findings to closure.
- [ ] Review public security claims before every release.
- [ ] Require security reviewer on sensitive changes.
- [ ] Run external review before remote beta.
- [ ] Run external review before agent-session beta.
- [ ] Run full external assessment before 1.0.

## Documentation checklist

- [ ] Maintain object-format specification.
- [ ] Maintain canonical-encoding specification.
- [ ] Maintain policy specification.
- [ ] Maintain sync-protocol specification.
- [ ] Maintain Git-projection specification.
- [ ] Maintain CLI reference.
- [ ] Maintain administrator guide.
- [ ] Maintain contributor architecture guide.
- [ ] Maintain migration guide.
- [ ] Maintain backup and recovery guide.
- [ ] Maintain security limitations page.
- [ ] Maintain release verification guide.

## Final project definition of done

- [ ] A team can initialize or import a repository.
- [ ] A team can work locally without Git.
- [ ] A team can synchronize through the native remote.
- [ ] A team can review and approve changes.
- [ ] A team can land changes without routine merge/rebase decisions.
- [ ] A team can publish signed audience-specific releases.
- [ ] A team can recover from ordinary mistakes and interrupted operations.
- [ ] A team can rotate keys and revoke future device access.
- [ ] Unauthorized clients cannot discover or decrypt restricted objects beyond configured redaction envelopes.
- [ ] Multiple human and agent sessions share project storage without sharing writable source state.
- [ ] Repository and protocol formats are documented and independently implementable.
- [ ] Existing Git repositories import with complete validation reports.
- [ ] Approved projections export to Git without protected-material leakage.
- [ ] Backup, restore, verify, repair, upgrade, and exit/export drills pass.
- [ ] Independent audits are complete.
- [ ] Public claims are bounded by the published threat model.
- [ ] Version 1.0 is signed, reproducible, documented, and supportable.
