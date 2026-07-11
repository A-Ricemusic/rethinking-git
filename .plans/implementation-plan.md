# Rethinking Git: Implementation Plan

Status: proposed
Target: a production-grade, permission-aware replacement for Git
Starting point: the Rust prototype in this repository
Planning horizon: approximately 18–30 months for a credible 1.0, depending on team size and security review cadence

## 1. Outcome

Build a version-control system that engineers can use instead of Git for normal daily development, while adding capabilities Git cannot safely provide:

- access control and encryption for individual objects, files, changes, operations, and releases;
- stable logical changes with automatic immutable snapshots;
- lightweight, isolated workspace sessions for humans and agents;
- one default integration operation instead of routine merge-versus-rebase decisions;
- signed, audience-specific releases;
- selective synchronization that does not send restricted readable objects to unauthorized clients;
- safe import, export, and mirroring for Git-based tools during migration.

The project is complete only when a team can initialize or import a repository, work locally, collaborate through a remote, review and land changes, publish releases, recover from mistakes, operate offline, and migrate away without losing data.

## 2. Scope and product boundaries

### Included in 1.0

- Local content-addressed object database
- Changes, snapshots, lines, typed markers, releases, conflicts, and operations
- Automatic snapshots and operation-level undo/redo
- File, object, and metadata policies
- Cryptographic identities, signatures, encrypted objects, and key rotation
- Permissioned local materialization and selective remote synchronization
- Isolated workspace sessions with shared immutable storage and safe build caches
- File-level three-way merge plus pluggable text and structural merge drivers
- Native remote service with atomic line updates
- Review, approval, CI status, and landing policy primitives
- Git repository import
- Public or explicitly authorized Git projection/export
- CLI, machine-readable CLI output, editor integration protocol, and agent SDK
- Repository maintenance: verification, compaction, garbage collection, backup, and recovery

### Deliberately excluded from the first 1.0

- A full GitHub-equivalent social product
- A general-purpose secret manager
- Guaranteed erasure of plaintext previously decrypted by an authorized device
- Hiding the content delta between two public releases
- Perfect semantic merging for every language
- Peer-to-peer key management as the only supported deployment model

For production secrets, the repository stores versioned references, schemas, and access policy. Direct encrypted values are supported for appropriate secrets, but high-value credentials should normally be resolved from a dedicated secret manager at materialization time.

## 3. Current repository baseline

The existing `src/main.rs` is an executable product model, not yet a production storage engine. It already proves:

- stable `Change` objects with base, target line, and current snapshot;
- immutable `Snapshot` objects with file manifests and blob hashes;
- content-addressed plaintext blobs under `.rgit/blobs`;
- `Line` heads and three-snapshot integration;
- persisted, permission-aware `Conflict` objects;
- actors, policy domains, path policies, and redacted views;
- permission-filtered status, diff, history, and operation output;
- an append-only operation record;
- unit coverage for policy propagation, merge behavior, and conflict visibility.

The prototype must not be incrementally stretched into production. Flat JSON files, whole-tree scans, plaintext blobs, simulated actors, non-transactional pointer updates, and one monolithic crate are intentionally temporary.

## 4. Engineering principles

1. **Objects are immutable.** Mutable names such as lines advance through compare-and-swap transactions.
2. **Authorization precedes discovery.** An unauthorized client must not learn restricted object identifiers unless policy explicitly allows a redacted envelope.
3. **Derived data inherits restrictions.** A merge, diff, index, build artifact, cache entry, conflict, or AI context cannot become less restricted without an explicit declassification operation.
4. **Every mutation is an operation.** Operations are durable, signed where relevant, reversible when semantics allow, and suitable for audit.
5. **The local workflow remains useful offline.** Network and identity outages must not destroy ordinary local work.
6. **The secure path is the easiest path.** CLI defaults cannot accidentally publish restricted objects.
7. **Compatibility is a boundary, not the data model.** Git import/export adapters must not force Git’s assumptions into native storage.
8. **Formats are specified and versioned.** Canonical encoding, object hashing, encryption envelopes, and protocols need independent specifications and test vectors.
9. **Security claims require evidence.** Threat models, adversarial tests, external audits, and reproducible builds gate releases.
10. **Performance is measured on real repositories.** Linux-scale history and large monorepos are continuous benchmark fixtures.

## 5. Target workspace structure

The first refactor creates a Cargo workspace with narrow crate responsibilities:

```text
crates/
  rgit-objects/       object types, canonical encoding, IDs, schema versions
  rgit-store/         local object DB, transactions, packs, indexes, GC
  rgit-crypto/        identities, signatures, AEAD, key envelopes, rotation
  rgit-policy/        capabilities, policy evaluation, derivation labels
  rgit-graph/         reachability, ancestry, diff, merge planning
  rgit-operations/    operation transactions, undo, recovery
  rgit-workspace/     watcher, snapshots, materialization, session overlays
  rgit-sync/          protocol types, negotiation, resumable transfer
  rgit-gitbridge/     Git import, export, remote-helper integration
  rgit-cli/           user-facing and machine-readable CLI
  rgit-server/        remote API, line transactions, audit, storage adapter
  rgit-testkit/       fixtures, model tests, failure injection
spec/
  objects.md
  canonical-encoding.md
  policy.md
  sync-protocol.md
  git-projection.md
```

Dependency direction is one-way:

```text
objects <- store, crypto, policy, graph
objects + store + policy <- operations, workspace, sync
all libraries <- cli, server, gitbridge
```

`rgit-cli` and `rgit-server` must contain orchestration only. Security and graph rules live in testable library crates.

## 6. Canonical object model

Before remote or encryption work, freeze a versioned logical schema. Exact binary encoding is decided in Milestone 1, but the semantic objects are:

### Blob

- plaintext content digest;
- byte length and chunk references for large content;
- content type hints that are non-authoritative;
- policy label reference;
- encrypted storage envelope stored separately from logical identity.

### Tree/manifest

- normalized path segments, never platform-dependent strings;
- entry type: file, directory, symlink, subproject, or secret reference;
- object reference, mode/executable bit, policy label;
- canonical sorted ordering;
- protection against path traversal, Unicode confusion, and case-fold collisions.

### Snapshot

- root manifest ID;
- parent snapshot IDs;
- owning change ID;
- authoring context and timestamps;
- policy label;
- optional message checkpoint;
- schema version.

### Change

- stable random ID;
- title and description objects;
- base snapshot and target line;
- current snapshot;
- author/owner identities;
- review and landing state;
- policy label.

### Line

- stable line ID and display name;
- head snapshot;
- generation counter for compare-and-swap;
- integration, approval, and release policies;
- visibility policy.

### Typed marker

- marker type: release, deployment, review, policy, or bookmark;
- target object;
- issuer and signature;
- policy and immutability rules.

### Release

- source line and exact source snapshot;
- audience;
- projected root manifest;
- build provenance and artifact references;
- semver or project-defined version;
- signatures, timestamp, and release policy decision.

### Operation

- operation ID, parent operation IDs, actor, and device;
- typed actions and their inverse/recovery data;
- public/redacted envelope and private payload references;
- policy label and signature;
- logical time plus informational wall-clock time.

### Policy label

- immutable policy ID and version;
- visibility rule;
- permitted actions: discover, read, materialize, derive, review, integrate, release, administer, audit;
- redaction behavior;
- declassification rule;
- key epoch.

Object IDs must be computed from canonical logical plaintext, while encrypted storage records may vary by recipient and key epoch. This preserves deduplication and graph integrity for authorized clients without equating ciphertext identity with logical identity.

## 7. Milestone plan

Each milestone ends in a runnable vertical slice. A milestone cannot close while its acceptance criteria are incomplete.

### Milestone 0 — Baseline and specifications

Estimated effort: 2–3 weeks
Primary result: a protected reference point and agreed invariants

Work:

- Add CI for formatting, Clippy, unit tests, dependency audit, and supported platforms.
- Record benchmark baselines for init, scan, snapshot, status, diff, and merge.
- Add end-to-end fixtures for public, mixed-policy, security-fix, and conflict flows.
- Write a threat-model skeleton: assets, actors, trust boundaries, attackers, metadata leakage, revocation limits, and recovery assumptions.
- Write architecture decision records for canonical encoding, hash algorithm agility, embedded database choice, async runtime, RPC framework, and cryptographic libraries.
- Document current prototype behavior as golden tests before refactoring.

Acceptance:

- Current CLI scenarios run in CI on Linux, macOS, and Windows where applicable.
- Golden tests capture all currently documented commands.
- Benchmark and threat-model documents are checked in.
- No behavioral refactor begins before these protections exist.

### Milestone 1 — Cargo workspace and production object store

Estimated effort: 5–8 weeks
Primary result: crash-safe local immutable storage

Work:

- Split object, graph, store, operation, and CLI code out of `src/main.rs`.
- Define canonical object encoding with explicit schema and format versions.
- Replace per-object JSON files with an embedded transactional metadata/index database plus immutable blob/pack files.
- Write objects through temporary files, fsync, atomic rename, and transaction commit in the correct order.
- Add object verification, repository consistency checking, repair reports, and schema migration framework.
- Add chunked storage for large files and streaming reads/writes.
- Implement reachability indexes and generation numbers for fast graph queries.
- Keep a debug command that renders canonical objects as JSON for inspection.

Acceptance:

- Power-loss/failure-injection tests cannot produce a line pointing at a missing object.
- Repeated writes deduplicate identical logical content.
- A repository can be copied and verified without the CLI’s in-memory state.
- Format test vectors decode identically across supported platforms.
- Existing prototype scenarios pass against the new store.

### Milestone 2 — Complete local version-control semantics

Estimated effort: 6–10 weeks
Primary result: a useful offline Git alternative for a single user

Work:

- Implement typed markers and immutable releases.
- Expand change lifecycle: start, describe, abandon, restore, split, combine, retarget, and list.
- Make automatic snapshots the default using a filesystem watcher plus command-boundary reconciliation.
- Implement operation transactions, operation graph, undo, redo, and recovery after interrupted commands.
- Add ignore and materialization rules with Git-compatible import of common patterns.
- Implement file mode, symlink, empty-directory policy, rename/copy heuristics, and subproject references.
- Add text patch display, binary handling, paging, color, stable machine-readable output, and shell completion.
- Add repository verify, compact, and garbage-collection commands with grace periods.
- Replace line integration’s write sequence with compare-and-swap and retryable integration transactions.

Acceptance:

- Editing files without manually snapshotting cannot lose a recoverable state.
- Every destructive-looking local action can be reversed through the operation log.
- Concurrent local CLI processes cannot corrupt repository state.
- The CLI supports ordinary solo development without invoking Git.
- Status and small-change snapshots meet defined latency budgets on benchmark repositories.

### Milestone 3 — Merge, conflicts, and landing engine

Estimated effort: 6–9 weeks
Primary result: `rgit land` replaces routine merge/rebase choices

Work:

- Generalize the existing three-snapshot merge planner into `rgit-graph`.
- Add line-based text merge with conflict regions stored as objects, not only path-level conflict flags.
- Add configurable drivers for JSON, TOML, YAML, lockfiles, generated files, and binary assets.
- Persist conflict sides, policy labels, assignments, candidate resolutions, and resolution provenance.
- Add virtual integration: compute a candidate result against the latest target without mutating either input.
- Add a local landing policy engine: required checks, allowed topology, ownership, signatures, and current-head precondition.
- Preserve stable change identity when recomputing ancestry.
- Add reusable resolution memory keyed conservatively by merge context.

Acceptance:

- `rgit land main` either atomically advances `main` or returns durable conflict objects.
- A concurrent line advance causes recomputation, never silent overwrite.
- Unauthorized users cannot see restricted conflict paths or sides.
- Merge fuzzing and property tests verify no unchanged input is lost.
- Expert topology controls exist, but the default flow never asks “merge or rebase?”

### Milestone 4 — Identity, capabilities, and encryption

Estimated effort: 8–12 weeks
Primary result: the local policy model becomes a cryptographic enforcement model

Work:

- Replace `--as <actor>` simulation with authenticated user, service, agent, and device identities.
- Introduce signed, scoped, expiring capabilities.
- Encrypt blobs, manifests, private change metadata, operation payloads, conflict data, and private indexes using authenticated encryption.
- Generate random data-encryption keys per object or bounded encryption group; wrap them for policy/key epochs.
- Implement key rotation and rewrapping without rewriting logical history.
- Separate discovery permission from content-read permission.
- Implement policy derivation labels for diff, merge, cache, build, search, and export results.
- Add explicit declassification operations requiring configured review and signatures.
- Design recovery keys, organization escrow options, device removal, and lost-key behavior.
- Prevent secrets from appearing in logs, error strings, crash reports, shell completion, or telemetry.

Acceptance:

- Copying server or local storage without keys does not reveal protected content or private metadata covered by the threat model.
- An unauthorized identity cannot enumerate restricted object IDs through supported APIs.
- Revoked devices cannot obtain new keys after the revocation point.
- Every derived-object creation path has a tested policy propagation rule.
- External cryptographic design review has no unresolved critical findings.

Security gate: do not market encrypted secret storage before this milestone passes external review.

### Milestone 5 — Permissioned workspace sessions

Estimated effort: 8–12 weeks, platform work may run in parallel
Primary result: many lightweight tasks or agents share one project without Git worktrees

Work:

- Define a workspace session as identity + change + view + environment profile + writable overlay.
- Build a portable materialization abstraction with platform backends:
  - Linux: overlayfs where available, FUSE fallback;
  - macOS: FUSE/provider or managed directory with copy-on-write reconciliation;
  - Windows: ProjFS or managed directory backend.
- Store one immutable local object pool per project and only session-specific deltas per workspace.
- Create a session-aware command broker so shells, editors, agents, and test runners receive the right view.
- Build safe content-keyed caches for package downloads, Rust artifacts, indexes, and generated dependencies.
- Key writable build caches by toolchain, lockfile, features, target, environment, and relevant policy label.
- Resolve authorized secret references into process environments or ephemeral mounts, not durable project files by default.
- Add lifecycle commands: create, enter, list, inspect, pause, resume, discard, and recover.
- Ensure lines such as `main` are never exclusively “checked out” by a workspace.

Acceptance:

- Two agents can edit different changes concurrently from one project without seeing or overwriting the other’s writable source state.
- Both sessions reuse safe dependency and build cache entries.
- Restricted files are absent or opaque in unauthorized materializations and never present in their backing overlay.
- Killing a broker or mount process does not lose captured work.
- Session startup is materially faster and smaller than a fresh Git worktree plus dependency install on benchmark projects.

### Milestone 6 — Native sync protocol and remote server

Estimated effort: 10–16 weeks
Primary result: multiple authorized users collaborate through a hosted repository

Work:

- Specify authenticated protocol negotiation, repository capabilities, and format versions.
- Implement authorization-before-discovery object negotiation.
- Implement resumable, streaming, integrity-checked uploads and downloads.
- Add atomic compare-and-swap line updates and idempotent operation publication.
- Support partial views, offline operation queues, reconnect reconciliation, and conflict reporting.
- Store encrypted objects separately from searchable policy/audit metadata with strict boundaries.
- Add organization identity integration, service accounts, agent identities, device enrollment, and capability issuance.
- Replicate an append-only signed audit log.
- Add quotas, rate limits, backup/restore, storage verification, and disaster-recovery drills.
- Add server-side garbage collection that respects every authorized view, release, retention rule, and in-flight transfer.
- Provide a single-node self-hosted deployment before distributed scaling.

Acceptance:

- Alice and Bob can synchronize different authorized views of the same logical line.
- The server never sends a restricted ciphertext/key combination that an unauthorized client can decrypt.
- Interrupted transfers resume without retransmitting completed chunks.
- Concurrent land attempts are serialized through line generation checks.
- Backup restoration preserves objects, line state, policy versions, keys, and auditable operations.
- The security-fix vertical slice works end to end across two clients, CI, and the server.

### Milestone 7 — Review, CI, and signed releases

Estimated effort: 8–12 weeks
Primary result: teams can govern changes without depending on a Git host

Work:

- Add published change state, reviewers, comments, approvals, ownership, and policy-aware diff views.
- Model CI results as signed attestations over exact snapshot and environment inputs.
- Add a landing queue that continuously recomputes candidate integrations against current line heads.
- Implement runner capabilities and prevent restricted jobs from running on unauthorized infrastructure.
- Implement release projections by audience, signed manifests, provenance, and reproducibility metadata.
- Add embargo scheduling and coordinated multi-audience publication.
- Add secret-reference resolution for authorized CI without embedding values in snapshots.
- Add audit views that reconstruct full provenance for authorized auditors while showing redacted public envelopes elsewhere.

Acceptance:

- A line can require ownership approval, tests, signatures, and policy checks before it advances.
- A release contains exactly the audience-approved projection and no restricted objects.
- CI attestations cannot be replayed against a different snapshot or environment.
- Embargoed security work remains unavailable to public clients until explicit release.
- The published release can be independently verified from its manifest and signatures.

### Milestone 8 — Git interoperability and migration

Estimated effort: 8–14 weeks
Primary result: real teams can adopt without a flag day

Work:

- Import Git commits, trees, blobs, refs, tags, authors, signatures, and supported attributes.
- Record deterministic mapping between Git object IDs and native object IDs.
- Map Git branches to lines/bookmarks according to an explicit import policy.
- Map commits to snapshots and infer stable changes where possible; never pretend inference is lossless.
- Export public or explicitly authorized audience projections as ordinary Git history.
- Refuse export when policy would leak restricted objects or metadata.
- Implement a Git remote helper or gateway for supported fetch/push workflows.
- Add one-way and two-way mirroring modes with conflict and force-update rules.
- Publish a detailed compatibility matrix for LFS, submodules, shallow clones, signatures, hooks, attributes, notes, replace refs, and unusual object forms.
- Build migration validation that compares trees, refs, authorship, and release markers.

Acceptance:

- Representative repositories import with identical checked-out public file state and verifiable mapping reports.
- Exported public projections are accepted by standard Git clients and hosting services.
- Restricted objects never appear in default exports, packfiles, commit messages, ref names, or mapping metadata.
- Teams can mirror to an existing Git remote while native clients use changes and workspace sessions.
- The migration tool reports every lossy or unsupported construct before cutover.

### Milestone 9 — Performance, reliability, and ecosystem

Estimated effort: 8–12 weeks, continuous throughout earlier milestones
Primary result: release-candidate quality

Work:

- Profile and optimize status, snapshotting, graph walks, policy evaluation, encryption, packs, and sync.
- Add commit-graph-like indexes, bloom/path filters, delta or chunk compression, and background compaction where measurements justify them.
- Test millions of objects, deep histories, large files, case-insensitive filesystems, slow disks, and high-latency networks.
- Add long-running concurrency, fault-injection, corruption, clock-skew, disk-full, and network-partition tests.
- Stabilize machine-readable CLI schemas and editor/agent APIs.
- Ship editor integration for change, diff, conflict, session, policy, and operation views.
- Publish SDKs/protocol bindings only after the wire and object specifications stabilize.
- Add opt-in, privacy-reviewed telemetry for latency and failure categories with no path or content capture.

Acceptance:

- Published performance budgets pass on Linux-scale history and selected monorepo fixtures.
- Crash and fault campaigns meet defined recovery objectives.
- Memory and local storage overhead are competitive with Git for public repositories.
- Restricted repositories have measured, documented encryption and selective-view overhead.
- CLI and integration APIs have compatibility guarantees and version negotiation.

### Milestone 10 — Security audit, beta, and 1.0

Estimated effort: 8–12 weeks plus remediation
Primary result: supportable public release

Work:

- Freeze 1.0 object, repository, and protocol formats.
- Complete independent cryptographic, authorization, server, client, and workspace-isolation audits.
- Run a private design-partner beta, then a public beta on non-critical repositories.
- Establish vulnerability reporting, coordinated disclosure, patch release, and key-compromise procedures.
- Publish backup, recovery, migration, operator, and incident-response documentation.
- Produce reproducible signed client and server releases with an SBOM.
- Prove full export so users can leave without vendor or format lock-in.
- Define support policy, upgrade windows, and backward compatibility.

Acceptance:

- No unresolved critical or high audit findings.
- Design partners complete real development and release cycles without Git as the source of truth.
- Restore and Git-export drills succeed from production-like backups.
- Upgrade from the previous supported format is tested and reversible.
- Public security claims match the threat model exactly.

## 8. Cross-cutting test strategy

### Unit tests

- Canonical encoding and hash stability
- Policy evaluation and capability scope
- Key envelope and rotation behavior
- Graph ancestry and reachability
- Diff and merge cases
- Operation inverses and recovery
- Path normalization and filesystem edge cases

### Property and model tests

- Object encode/decode round trips
- Store transactions against an in-memory reference model
- Merge invariants: unchanged content is preserved; identical changes converge
- Policy monotonicity: derivation never weakens restrictions implicitly
- Sync convergence under reordered, duplicated, and interrupted messages
- GC never removes reachable or retention-protected objects

### Fuzzing

- All object and protocol decoders
- Pack/chunk readers
- Path and manifest parsing
- Git import parser boundaries
- Merge drivers
- Policy and capability tokens

### Adversarial integration tests

- Unauthorized enumeration attempts
- Ciphertext swapping and rollback
- Malicious object graphs and decompression bombs
- Restricted path leakage through errors, search, conflict UI, and telemetry
- Compromised or stale client capabilities
- Concurrent line updates
- Symlink and path traversal during materialization
- Cache poisoning across policy domains

### Failure injection

- Process kill between every transaction phase
- Disk full and partial writes
- Corrupted index or object data
- Key service unavailable
- Server unavailable during publish
- Network partition during line update
- Watcher event loss and filesystem races

## 9. Security workstream

Security is not a late milestone owned by one engineer. Assign a security lead from Milestone 0 and maintain these artifacts continuously:

- threat model and trust-boundary diagrams;
- cryptographic construction document;
- metadata-leakage inventory;
- capability and authorization matrix;
- key lifecycle and recovery runbook;
- policy-derivation rules for every object-producing subsystem;
- security test corpus;
- audit finding tracker;
- public security claims and limitations.

Required external reviews:

1. Object identity and cryptographic design before Milestone 4 closes.
2. Sync authorization and key distribution before external remote beta.
3. Workspace isolation and secret materialization before agent-session beta.
4. Full client/server assessment before 1.0.

## 10. Performance budgets

Set exact hardware profiles in Milestone 0. Initial product targets are:

- warm status for a small repository: perceived instantaneous;
- warm status for a large monorepo: under one second for ordinary changes;
- automatic snapshot: background and non-blocking for normal edits;
- workspace session creation with warm caches: seconds, not dependency-install minutes;
- incremental sync: proportional to changed authorized objects, not full history;
- line integration preview: interactive for normal changes;
- object verification and GC: resumable background operations.

These are hypotheses until benchmark hardware and fixtures are checked in. Do not optimize around synthetic microbenchmarks at the expense of correctness or security.

## 11. Team and sequencing

A practical core team for the main build is 8–12 engineers plus design/product and recurring security review:

- 2 storage/graph engineers;
- 2 security/identity engineers;
- 2 workspace/platform engineers;
- 2 sync/server engineers;
- 1 Git interoperability engineer;
- 1 CLI/developer-experience engineer;
- shared test/reliability and infrastructure ownership.

Milestones 0–2 should be mostly sequential because they establish the model and store. After Milestone 2:

- merge/landing can proceed alongside identity/encryption;
- platform-specific workspace backends can proceed in parallel;
- remote protocol specification can begin while the crypto model stabilizes, but implementation must not freeze before authorization review;
- Git import can begin early, while export must wait for policy-safe projection semantics;
- performance and fault testing run continuously.

## 12. Issue and pull-request breakdown

Each milestone should be split into issues that deliver a testable slice rather than horizontal scaffolding. Every implementation issue includes:

- user-visible or invariant-level outcome;
- affected object/format versions;
- policy and metadata-leakage analysis;
- failure behavior and recovery path;
- unit, property, integration, and benchmark coverage as applicable;
- migration impact;
- documentation change.

Pull requests should avoid combining schema changes, storage changes, CLI redesign, and format migrations unless the slice cannot work otherwise. Security-sensitive APIs require a second reviewer from the security workstream.

## 13. First 12 implementation issues

These are the recommended first issues after approving the plan:

1. Add golden CLI tests for every flow in `docs/prototype.md`.
2. Add cross-platform CI, Clippy, formatting, audit, and benchmark jobs.
3. Write ADRs for canonical encoding, database, hashes, crypto libraries, async runtime, and RPC.
4. Create `rgit-objects` with versioned IDs and canonical test vectors.
5. Create `rgit-store` with atomic immutable-object writes and an in-memory test backend.
6. Add transaction failure injection and crash-consistency model tests.
7. Move change, snapshot, line, conflict, and operation types out of `main.rs` without changing CLI behavior.
8. Implement repository verification over the new store.
9. Create `rgit-graph` and move manifest diff and three-way merge behind library APIs.
10. Implement compare-and-swap line advancement as one operation transaction.
11. Add typed marker and release object schemas with CLI read-only inspection.
12. Replace direct CLI printing in libraries with typed results and stable output adapters.

The first vertical checkpoint is reached when the existing demo works on the new crash-safe object store and produces byte-stable canonical test vectors.

## 14. Project-level definition of done

The replacement is ready for 1.0 when all of the following are true:

- A new team can use it for daily work, review, integration, release, and recovery without Git as source of truth.
- Unauthorized clients cannot discover or decrypt restricted objects beyond explicitly configured redaction envelopes.
- Audience-specific releases contain only approved projected content and carry verifiable provenance.
- Multiple human and agent sessions share project storage and warm caches without sharing writable source state.
- `land` handles the normal integration path without requiring users to choose merge or rebase.
- Local operations remain useful offline and synchronize safely later.
- Repository, object, protocol, and CLI formats are documented, versioned, tested, and externally implementable.
- Existing Git repositories can be imported with a complete validation report.
- Approved projections can be exported to Git without leaking protected material.
- Backup, restore, verify, repair, upgrade, key rotation, revocation, and full exit/export have been exercised.
- Security audits are complete and all public claims are bounded by the published threat model.

## 15. Immediate decision gate

Before writing production code, approve or revise these five decisions:

1. Keep `change + snapshot + operation` as the permanent core model.
2. Treat `view` and policy labels as storage/sync security boundaries, not UI filters.
3. Make workspace sessions isolated overlays; do not promise two writable branch states in one ordinary filesystem namespace.
4. Make releases signed audience projections rather than aliases for line heads.
5. Build Git interoperability as an adapter while keeping the native object model independent.

If these decisions are accepted, begin Milestone 0 and the first 12 issues above. Do not begin remote hosting, a GUI, or broad command expansion until canonical objects, transaction safety, and threat-model invariants are in place.
