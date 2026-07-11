# RGit Threat Model, Version 0

Status: initial review baseline

Last updated: 2026-07-11

Review cadence: at every milestone and before each security-sensitive release

## 1. Security objectives

RGit is a permission-aware version-control system. Its primary security claim is
narrow: an actor must be authorized before discovering, receiving, decrypting,
materializing, deriving from, or publishing protected repository objects. Encryption
at rest supports this claim but does not replace server-side authorization.

The system also aims to provide:

- integrity and origin authentication for immutable objects, operations, policy,
  releases, and mutable-reference transactions;
- crash-safe, rollback-detectable local and remote state transitions;
- least-privilege workspace and automation materialization;
- policy propagation to diffs, conflicts, indexes, caches, logs, builds, reviews, and
  audience-specific releases;
- recoverability without weakening access control or destroying audit evidence.

RGit does **not** promise to erase plaintext already observed by an authorized party,
hide differences between public releases, protect a fully compromised endpoint while
it is authorized and materializing data, or replace a dedicated production secret
manager.

## 2. Assets

| Asset | Required properties |
| --- | --- |
| Source, configuration, secret values, exploit reproductions | Confidentiality by policy; integrity; durable availability. |
| Object IDs and graph relationships | Confidential when discovery is denied; integrity and anti-substitution. |
| Paths, sizes, authors, timestamps, messages, policy names | Confidential according to metadata/redaction policy. |
| Identities, memberships, capabilities, policy versions | Authenticity, freshness at decision boundary, auditability. |
| Signing and decryption private keys | Confidentiality, non-exportability where supported, rotation/recovery. |
| Lines, change heads, operation heads, release state | Atomicity, integrity, rollback/fork detection. |
| Operation/audit history and approval/CI evidence | Completeness within the authorized view, integrity, accountability. |
| Workspace plaintext, temporary files, editor state, caches | Confidentiality matching source policy; prompt deletion on dematerialization. |
| Backups, packs, indexes, transfer resumptions | Same or stricter policy as their inputs; recoverability. |
| Availability of repository and key services | Bounded denial-of-service resistance and documented recovery. |

## 3. Actors and components

### Human and service actors

- repository readers, contributors, reviewers, release managers, policy
  administrators, and auditors;
- external contributors who receive only a public projection;
- CI, build, deployment, indexing, editor, and agent identities;
- repository and organization administrators, who are privileged but not implicitly
  entitled to decrypt every policy domain;
- recovery custodians holding threshold recovery material.

### Trusted components

Trusted for all security properties they enforce:

- canonical object validation and ID/signature verification;
- policy evaluator and authorization middleware;
- client cryptographic/key agent and OS random-number generator;
- workspace materializer and policy-aware derivation tracker;
- transaction coordinator for durable object/reference publication;
- identity, membership, policy, and key-envelope authorities;
- release projector and signer for the audience it serves;
- trusted display path for security-critical confirmations.

Trusted for integrity/availability but **not plaintext confidentiality**:

- native sync service after authorization routing (it may store only ciphertext for
  protected domains, although metadata visible to it remains exposed);
- embedded metadata database and immutable pack storage;
- object storage, backup system, CDN, and transport proxies.

Conditionally trusted:

- CI, merge drivers, editors, language servers, agents, and build tools only for the
  exact capabilities and materialized domains granted to their dedicated identities;
- secret-manager providers for values resolved through `SecretRef`;
- Git bridges for explicitly projected public or authorized content. A Git remote is
  assumed unable to preserve native confidentiality semantics.

No individual storage administrator, database snapshot, or pack file is trusted to
grant plaintext access. Administrators controlling identity/policy authorities can
grant future access and must be protected by separation of duties and audit.

## 4. Trust boundaries

1. **User/CLI boundary:** terminal, shell, hooks, environment, clipboard, pager, and
   editor can observe output. Machine-readable output remains policy-filtered.
2. **Process/host boundary:** plaintext crosses from encrypted local storage through
   the key agent into workspace, memory, compiler, editor, and caches.
3. **Client/server boundary:** mutually authenticated encrypted transport ends;
   authorization must precede object negotiation and existence responses.
4. **Service/storage boundary:** sync and key services write ciphertext, indexes,
   database state, object storage, backups, and logs operated by different principals.
5. **Policy-domain boundary:** data derived from multiple domains adopts the least
   permissive effective policy unless an authorized declassification is recorded.
6. **Native/Git boundary:** native objects become a flattened Git projection and lose
   object-level policy, private history, operation semantics, and revocation.
7. **Repository/secret-provider boundary:** secret references are versioned; resolved
   values leave the provider only for an authorized process and must not be captured.
8. **Workspace/session boundary:** sessions share immutable encrypted objects but not
   writable overlays; shared build caches require policy partitioning.
9. **Backup/recovery boundary:** keys and state enter separately controlled recovery
   media and procedures; restoration must not reactivate revoked credentials.
10. **Release/public boundary:** a signed audience projection becomes intentionally
    visible and cannot later be made confidential.

Every implementation data-flow diagram must identify which boundary a new component
crosses, its authenticated principal, plaintext exposure, logged metadata, and failure
behavior. “Internal network” is not a trust boundary exemption.

## 5. Attacker profiles

- unauthenticated remote attacker probing repository existence, IDs, timing, and
  parser/resource limits;
- authenticated actor with narrow access attempting horizontal or privilege
  escalation, graph traversal, confirmation attacks, or confused-deputy use;
- malicious contributor submitting crafted paths, objects, packs, merges, hooks, or
  build inputs;
- revoked or former member retaining previously synchronized ciphertext/plaintext;
- compromised endpoint, editor, agent, CI runner, or dependency operating with its
  current permissions;
- storage, backup, CDN, database, or network operator reading/tampering with records;
- malicious or compromised repository/policy administrator;
- supply-chain attacker influencing dependencies, toolchains, builds, updates, or
  cryptographic parameter negotiation;
- local unprivileged process observing filesystem permissions, process arguments,
  environment, temporary files, sockets, or shared caches;
- availability attacker causing resource exhaustion, transaction contention, partial
  transfers, disk-full conditions, rollbacks, or recovery loops.

We assume modern cryptographic primitives remain secure, trusted endpoints verify
released binaries, the OS enforces process/filesystem boundaries, and at least the
required recovery quorum remains honest. These assumptions are tested and narrowed
over time; they are not guarantees supplied by RGit.

## 6. Threats and required controls

### Unauthorized discovery and read

- Evaluate `discover` before returning existence, IDs, paths, ancestry, counts, or
  authorization-distinguishing errors. Batch and pad sensitive responses where
  practical; specify unavoidable channels.
- Evaluate `read` independently before sending a storage envelope. Capability tokens
  are audience-, repository-, action-, policy-version-, and expiry-bound.
- Encrypt protected object payloads with per-object data keys and authenticated
  context binding repository, object ID, policy, epoch, and algorithm suite.
- Never place plaintext/object IDs into unrestricted logs, metrics, traces, crash
  reports, error messages, shell arguments, or telemetry.
- Treat IDs as confidential metadata where discovery is denied; hashing predictable
  content does not provide confidentiality.

### Policy bypass and confused deputy

- Resolve actor, device, membership, policy version, action, and resource in one
  authorization decision; default deny and make denials dominant.
- Bind delegated capabilities to a least-privilege service identity. Human credentials
  must not be silently forwarded to agents, hooks, merge drivers, or CI.
- Make every derived object inherit the join of all input restrictions. Declassification
  is an explicit signed operation requiring the configured approvals.
- Check policy both at request admission and transaction commit to close time-of-check
  races. Long transfers use short leases and recheck before publication.

### Integrity, rollback, and equivocation

- Verify canonical encoding, kind, schema, object digest, signatures, and referenced
  kinds before admitting untrusted objects.
- Use compare-and-swap generations for lines and other mutable references. Bind
  operations to before/after state and reject stale updates.
- Authenticate storage envelopes and transport. Do not negotiate downgrade to an
  unapproved hash, signature, AEAD, or protocol suite.
- Maintain signed operation/reference checkpoints and surface inconsistent server
  views. Transparency anchoring and witness quorum remain future hardening work.

### Malicious content and filesystem escape

- Reject traversal, absolute paths, NUL, separators inside segments, normalization
  and case-fold collisions, reserved names under the portable profile, unsafe symlink
  targets, and unexpected special files.
- Parse canonical objects, packs, diffs, and protocol frames with strict size, depth,
  count, decompression-ratio, CPU, and allocation budgets; fuzz all parsers.
- Hooks and merge/build tools execute outside the trusted core with explicit inputs,
  outputs, network policy, timeouts, and service identity.

### Endpoint plaintext leakage

- Materialize only requested paths allowed for the session identity. Use owner-only
  permissions and avoid system temporary directories, command arguments, and global
  caches.
- Partition derived/build caches by effective policy and cryptographic tenant. Cache
  hits must never reveal restricted object existence.
- Reconcile and securely dematerialize best-effort on capability loss. Document that
  journaling filesystems, swap, backups, editors, SSD behavior, and compromised hosts
  prevent guaranteed physical erasure.
- Secret values resolve as late as possible into process-specific channels and are
  excluded by snapshot scanning independent of ignore rules.

### Key compromise and cryptographic misuse

- Separate signing, device decryption, release, service, and recovery keys. Prefer OS
  key stores/hardware-backed keys; never store private keys in repository objects.
- Rotate by policy epoch and rewrap data keys without changing logical object IDs.
  Rotation does not require rewriting plaintext objects.
- Reject nonce reuse by construction, bind algorithm identifiers into authenticated
  context, use CSPRNG output, and zeroize ephemeral secret buffers where practical.
- Dependency, test-vector, audit, and key-ceremony requirements are release gates.

### Denial of service and availability

- Apply authenticated quotas for object count/size, graph traversal, negotiation,
  concurrent streams, database time, and temporary disk use.
- Stream and incrementally verify large objects; do not allocate from attacker-stated
  lengths without caps. Garbage collection never races uncommitted transactions.
- Preserve last-known-good signed reference checkpoints and support read-only degraded
  operation. Recovery reports are non-destructive by default.

## 7. Metadata leakage inventory

| Channel | Possible leakage | Baseline treatment |
| --- | --- | --- |
| Object IDs/deduplication | Confirmation of guessed content; equality across views | Suppress before discovery; domain-separated IDs; no cross-policy dedup signal in APIs. |
| Manifest/graph shape | Hidden path count, ancestry, private integration | Audience projection omits restricted references; use opaque/redacted shells only by policy. |
| Ciphertext | Approximate size, equality, key epoch, access frequency | Pad size classes where configured; randomized envelopes; segregate domains; record residual risk. |
| Network | Repository membership, transfer volume/timing, change cadence | TLS, batching, resumable chunks; traffic analysis remains residual risk. |
| Errors/timing | Existence oracle and policy membership | Uniform unavailable response and bounded authorization paths; continuously test. |
| Logs/metrics/traces | Actor, path, object ID, operation intent | Structured allowlist logging; policy-aware sinks; short retention; no plaintext. |
| Local filesystem | Names, sizes, timestamps, editor backups, caches | Restricted workspace permissions and policy-partitioned caches; endpoint risk documented. |
| Notifications/review/CI | Private title, author, status, diff, artifact | Render through recipient view; dedicated service identity; no human-token forwarding. |
| Git export/release | Flattened history and delta between public versions | Explicit signed projection and confirmation; publication is irreversible. |
| Backup/recovery | Historical ciphertext and obsolete key envelopes | Separate encryption/control plane, retention, tested destruction, epoch-aware restore. |

Each protocol and feature review must update this table with new observable fields.
Security tests require two repositories that are indistinguishable to an unauthorized
client except for documented traffic-analysis leakage.

## 8. Revocation and previously decrypted plaintext

Revocation prevents new authorization decisions, key-envelope issuance, materialization,
and accepted signatures after its effective boundary. It can trigger session closure,
credential/token invalidation, key-epoch advance, data-key rewrapping for remaining
members, and best-effort workspace/cache cleanup.

Revocation cannot:

- make an actor forget plaintext, screenshots, copied object IDs, or public releases;
- reliably erase data from SSDs, swap, editor history, host backups, or a compromised
  device;
- repair credentials embedded in old source or invalidate credentials at an external
  provider;
- hide the delta between old and new public artifacts.

Therefore high-value credentials use `SecretRef` and provider-side rotation. Highly
sensitive fixes minimize audience and embargo duration. Incident response assumes
that data materialized to a compromised/revoked device may be lost. UI and policy
language must never imply retroactive secrecy.

Offline clients receive time-bounded capabilities. They may continue operations
within the signed offline lease and cached keys; strict immediate revocation and
unbounded offline use are incompatible. The maximum offline window is a deployment
policy and must be visible to administrators.

## 9. Recovery assumptions and procedures

- Immutable object ciphertext is backed up independently from metadata/reference
  checkpoints and independently from key/recovery material.
- Recovery keys use an organization-defined quorum; no routine service stores the
  complete recovery capability. Ceremonies are logged and periodically exercised.
- Backups are authenticated, encrypted, versioned, and restored into quarantine.
  Restore verifies every digest/signature/reference before any mutable reference is
  published.
- Restored policy and identity state is advanced to current revocation/key epochs;
  obsolete backups never reactivate revoked principals.
- A database can be rebuilt from verified immutable objects plus signed operation and
  reference checkpoints. Missing objects produce a non-destructive report.
- Loss of all authorized decryption and recovery keys means protected plaintext is
  intentionally unrecoverable. This is documented during repository setup.
- Disk-full, crash between publication phases, corrupted indexes, stale replicas,
  partial packs, and interrupted key rotation are mandatory failure-injection cases.
- Emergency access is explicit, time-limited, quorum-approved, audited, and does not
  suppress ordinary authorization logs.

Recovery objectives (RPO/RTO), geographic redundancy, retention, and operator runbooks
are deployment decisions and must be set before production use.

## 10. Security verification gates

Before security claims reach production:

- unit and property tests cover canonicalization, policy lattice/derivation, capability
  scope, envelope context, and reference transactions;
- fuzzers cover every untrusted parser and graph/resource limit;
- adversarial integration tests use actors with overlapping and revoked policies and
  assert both content and metadata non-disclosure;
- fault injection covers every durable publication and rotation phase;
- dependencies are pinned, reviewed, vulnerability/license checked, and reproducible;
- protocol and cryptographic code receives independent expert review and an external
  audit before 1.0;
- findings receive owners, severity, remediation, regression tests, and disclosure
  handling. Threat-model changes are part of each design review.

## 11. Open risks requiring later decisions

- policy language and group-membership evaluation time semantics;
- traffic-analysis defenses and size-padding profiles;
- server equivocation detection/transparency witnesses;
- offline lease defaults and emergency revocation UX;
- hardware-backed key support matrix and recovery quorum protocol;
- endpoint sandboxing guarantees for plugins, agents, merge drivers, and CI;
- formal derivation-label algebra and projection-proof construction;
- multi-tenant side-channel isolation and policy-safe global deduplication;
- denial-of-service budgets for Linux-scale and monorepo graphs.

These items do not weaken the baseline rule: until specified, the implementation must
choose the more restrictive behavior or keep the feature disabled.
