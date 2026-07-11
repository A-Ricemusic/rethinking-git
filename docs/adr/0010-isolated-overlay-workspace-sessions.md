# ADR 0010: Isolated overlay workspace sessions

- Status: Proposed
- Date: 2026-07-11
- Owners: workspace and security (formally unassigned)

## Context

Git worktrees duplicate materialized source trees, fragment build/dependency state, and
make a branch exclusive to whichever worktree checked it out. Letting two agents write
different logical changes in one ordinary directory is more dangerous: a pathname has
only one visible value, while editors, compilers, language servers, shell processes, and
file watchers have no reliable way to infer which agent's branch a read or write belongs
to.

The product needs concurrent lightweight sessions that share immutable repository data
and safe caches without sharing writable source state. Permissioned views add a harder
requirement: unauthorized plaintext must not be copied into a common lower layer, page
cache, build cache, index, log, or fallback directory exposed to another session.

## Decision

A workspace session is the tuple `identity + change + view + environment profile +
writable overlay`. Each session receives a distinct filesystem root and process context.
All sessions share one immutable, content-addressed project object pool; they share cache
entries only when the complete cache key and policy permit it. Session roots contain or
present only authorized materialization plus session-specific writable deltas.

Lines such as `main` are compare-and-swap graph references, never exclusively checked
out resources. A session records its base snapshot and target change, so any number of
sessions may start from `main` without preventing another session or the ordinary project
directory from doing so.

The platform abstraction has these backends:

- Linux prefers kernel overlayfs when capability probes and mount policy allow it, with
  a brokered FUSE backend as the virtual-filesystem fallback.
- macOS prefers a brokered filesystem provider/FUSE backend when an approved provider is
  installed; otherwise it uses the portable managed-directory backend.
- Windows prefers ProjFS when available and enabled; otherwise it uses the portable
  managed-directory backend.
- The portable managed-directory backend materializes authorized immutable files with
  reflinks or copies where safe and performs copy-on-write reconciliation before a file
  becomes writable. It may consume more disk and startup time but preserves semantics.

Hard links are never used where a session write could mutate shared content. Backend
selection follows runtime capability probes and is visible in session inspection; it
must not silently weaken access policy or durability.

A session-aware command broker creates, enters, pauses, resumes, recovers, and discards
sessions; gives child processes the session root and identity-scoped environment; and
reconciles overlay deltas into recoverable snapshots at command boundaries and during
shutdown/checkpointing. Broker or mount failure leaves a recoverable delta journal and
immutable objects, not the only copy of unrecorded work.

## Security boundary

The authorization boundary is the view derivation and materialization broker, not the
overlay mechanism alone. Restricted objects are decrypted only after identity,
capability, view, and path policy checks. Unauthorized paths are absent or represented
by an explicitly non-content-bearing opaque entry; their plaintext is never installed
in that session's lower layer or writable overlay. Secret references resolve into
identity-scoped process environments or ephemeral mounts by default, not durable source
files.

Session directories, journals, and ephemeral mounts use owner-only permissions. Cache
keys include content/toolchain inputs and the effective policy/view partition; restricted
outputs cannot populate public cache partitions. Logs, errors, watcher events, indexes,
core dumps, swap, backups, antivirus, and crash artifacts are part of the leakage review.
Session teardown revokes broker access and removes ephemeral plaintext on a best-effort
basis, but cannot revoke plaintext already observed or copied by an authorized process.

Filesystem separation is protection against accidental cross-session reads/writes and
cooperative tools. It is not a sandbox against malicious processes running as the same
OS user: such a process may bypass the session root and inspect peer processes or files.
Adversarial agent isolation requires an additional OS sandbox, container, VM, or distinct
OS identity. Product claims and tests must preserve this distinction.

## Failure and fallback behavior

If the preferred backend is unavailable, capability probing selects the portable
managed-directory backend and reports the reason. Session creation fails closed rather
than materializing content when authorization, safe path handling, owner-only
permissions, journal creation, or policy-partitioned caching cannot be guaranteed. A
backend failure remounts/reopens only after journal reconciliation; it never treats a
partial materialization as a valid snapshot.

Network filesystems and cloud-synchronized folders remain outside the 1.0 production
profile under [ADR 0009](0009-supported-platforms-and-filesystems.md). Platform-specific
backends must produce the same snapshot semantics and pass a shared conformance suite.

## Consequences

- Agents get independent writable namespaces without cloning repository objects or
  making a line exclusive, while caches and immutable objects can be reused safely.
- Every active session still needs a distinct mount point or managed root; this design
  reduces duplication but does not promise zero materialization cost.
- Virtual filesystem backends add privileged/platform integration and failure modes.
  The slower managed backend is required to keep the model portable and debuggable.
- Tools must be launched in a session context. Processes aimed at a different session
  root see that root's state, exactly as normal filesystem semantics require.
- Build cache sharing is conditional, not automatic; confidentiality takes precedence
  over hit rate.

## Rejected alternatives

- **Two writable branch/change states in one ordinary directory.** Path reads and writes
  cannot be attributed reliably across arbitrary tools, so agents would overwrite or
  observe one another's state. Process interposition cannot cover every syscall, editor,
  compiler, child process, or crash.
- **One complete Git-style worktree per session.** Semantically safe, but duplicates
  materialization, repeats setup, and preserves branch checkout exclusivity.
- **Containers as the only session primitive.** Stronger isolation, but too heavy and
  unavailable for some local workflows. Containers remain an optional security layer.
- **A shared writable source tree with advisory locks.** Locks serialize useful work,
  are routinely bypassed by ordinary tools, and do not provide confidentiality.
- **Always require a kernel/provider backend.** Excludes supported machines and makes
  recovery dependent on optional privileged software.

## Verification and open work

Build one backend-neutral conformance suite covering concurrent writes, rename/delete,
symlinks and path traversal, case/Unicode collisions, watcher loss, broker/mount death,
disk full, interrupted reconciliation, and snapshot equivalence. On every Tier 1 OS,
prove two sessions cannot see or overwrite one another's deltas, restricted plaintext
never appears in an unauthorized lower/upper layer or cache, killed processes preserve
captured work, and fallback selection is explicit. Benchmark startup, disk use, cache
reuse, and incremental reconciliation against a fresh Git worktree plus dependency
setup.

Before changing this ADR to `Accepted`, assign workspace and security owners, review the
threat model and product isolation claims, prototype at least the managed fallback on a
Tier 1 platform, and obtain accountable-role approval under the
[milestone review process](../governance/milestone-reviews.md). Independent security
review is additionally required before an agent-session beta.
