# Prototype

This is the first Rust prototype for the `jj`-inspired model.

It implements the first source-control primitives:

- `change`: the stable logical unit of work
- `snapshot`: an immutable capture of the repository files
- `workspace`: the current editable context
- `operation`: an append-only record of source-control actions
- `line`: a shared integration target such as `main`
- `actor`: a person or tool with domain grants
- `path policy`: file-level access control

There are no branches, tags, remotes, real encryption, or hosting yet.

## Install Rust

If `cargo` or `rustc` are not available, install Rust.

Install Rust with:

```sh
brew install rustup
PATH="/opt/homebrew/opt/rustup/bin:$PATH" rustup toolchain install stable
```

Then check:

```sh
PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo --version
PATH="/opt/homebrew/opt/rustup/bin:$PATH" rustc --version
```

If you want `cargo` and `rustc` available in every new shell:

```sh
echo 'export PATH="/opt/homebrew/opt/rustup/bin:$PATH"' >> ~/.zshrc
```

After restarting the shell, these should work:

```sh
cargo --version
rustc --version
```

## Build

```sh
cargo build
```

## Flow

Initialize the repo:

```sh
cargo run -- init
```

Create a logical change:

```sh
cargo run -- change new add-dark-mode-settings
```

Check what has changed since the latest snapshot:

```sh
cargo run -- status
```

Show the workspace diff:

```sh
cargo run -- diff workspace
```

Create a snapshot:

```sh
cargo run -- snapshot --message "add settings toggle"
```

Inspect the workspace:

```sh
cargo run -- workspace info
```

List changes:

```sh
cargo run -- change list
```

Show a change:

```sh
cargo run -- change show chg_example
```

List snapshots:

```sh
cargo run -- snapshot-info list
```

Show a snapshot:

```sh
cargo run -- snapshot-info show snap_example
```

Show the operation log:

```sh
cargo run -- op log
```

## Permissioned Flow

Initialize the repo:

```sh
cargo run -- init
```

Create actors:

```sh
cargo run -- actor set alice --domain public
cargo run -- actor set bob --domain public --domain team/security
cargo run -- actor set admin --domain public --domain admin
```

Restrict sensitive paths:

```sh
cargo run -- access path .env --domain admin
cargo run -- access path security --domain team/security
```

Create a private security change:

```sh
cargo run -- change new fix-token-replay --domain team/security
```

Create files:

```sh
mkdir -p src security
printf 'patched auth\n' > src/auth.txt
printf 'SECRET=value\n' > .env
printf 'exploit repro\n' > security/repro.test
```

Create the snapshot:

```sh
cargo run -- snapshot --message "fix token replay"
```

Bob can review the security material, but cannot preview or integrate this
snapshot because it also contains the admin-only `.env` file:

```sh
cargo run -- snapshot-info list --as bob
cargo run -- snapshot-info show snap_example --as bob
cargo run -- merge preview --into main --as bob
cargo run -- line integrate main --as bob
```

An actor must be authorized for every file in a snapshot before integrating it.
Admin can inspect the complete snapshot and integrate it into `main`:

```sh
cargo run -- merge preview --into main --as admin
cargo run -- line integrate main --as admin
```

Alice can see the shared line but not restricted files:

```sh
cargo run -- line view main --as alice
cargo run -- change list --as alice
cargo run -- op log --as alice
```

Bob can see the security material on the integrated line, while `.env` remains
hidden:

```sh
cargo run -- line view main --as bob
cargo run -- change list --as bob
cargo run -- snapshot-info list --as bob
cargo run -- op log --as bob
```

Admin can see everything, including `.env`:

```sh
cargo run -- line view main --as admin
```

Show line history with actor-specific redaction:

```sh
cargo run -- line history main --as alice
cargo run -- line history main --as bob
cargo run -- line history main --as admin
```

Show actor-filtered diffs:

```sh
cargo run -- diff workspace --as alice
cargo run -- diff snapshot snap_old snap_new --as bob
cargo run -- diff line main --as admin
```

Diffs currently show file-level added, modified, deleted, and hidden restricted counts. They do not show line-level text patches yet.

## Merge And Conflict Flow

Preview whether the current change can integrate into `main`:

```sh
cargo run -- merge preview --into main --as alice
cargo run -- merge preview --into main --as bob
```

Integrate still uses the line command:

```sh
cargo run -- line integrate main --as bob
```

If the target line changed since the change started, integration runs a three-snapshot merge:

```text
base snapshot + current line snapshot + incoming change snapshot
```

If the same path changed on both sides, integration stores a conflict and refuses to update the line:

```sh
cargo run -- conflict list --as alice
cargo run -- conflict list --as bob
cargo run -- conflict show conf_example --as bob
```

Conflict output is permission-aware. Actors only see conflicts where they can access the line, change, and every file side involved in the conflict.

### Exit status

Successful commands return `0`; command refusals and repository errors return `1`;
invalid CLI syntax returns `2`. Direct change, snapshot, conflict, and line reads
return `1` with `Error: operation unavailable` on stderr and no stdout when the
requested object is missing or restricted. Restricted workspace status/diff and
snapshot comparisons use that same refusal. Parsing/corruption errors remain errors,
not successful empty output. Empty visible lists and a workspace without a current
change are successful inspections. This corrects earlier successful exit statuses for
denied reads; automation must check the exit status before consuming output.


`merge preview` and `line integrate` return exit code `1` when authorization
prevents the operation. `line integrate` also returns `1` after it stores a merge
conflict and leaves the line unchanged. Successful previews and integrations return
`0`.

The permission-aware explanation and visible conflict details remain on standard
output for interactive use. Standard error contains only a generic
`operation unavailable` or `integration blocked by conflicts` message so automation
can detect the refusal without receiving restricted object details.

## Storage

The prototype stores state in `.rgit/`.

```text
.rgit/
  repo.json
  workspace.json
  path-policies.json
  actors/
  blobs/
  changes/
  lines/
  operations/
  snapshots/
```

Snapshots reference blobs by SHA-256 hash.

Changes point at their current snapshot.

The workspace points at the current change.

Operations record how state changed over time.

Actors and path policies decide which objects are visible in commands that accept `--as`.

This is not cryptographic security yet. It is the local policy and view model that real encrypted sync would enforce later.

## Metadata visibility

Actor-filtered commands redact references to snapshots and changes whose metadata the actor cannot read. A visible change does not grant access to a private snapshot message or parent ID. Public line views can still show permitted files while displaying `restricted` for the integration snapshot.

`rgit workspace info` now defaults to the public actor; use `rgit workspace info --as admin` for the administrative view. These views remain local policy simulations, not authenticated access control.

## Snapshot fidelity limits

Snapshots preserve UTF-8 filenames exactly, including spaces and literal Unix backslashes. Access path policies use host path separators and repository-relative paths; they reject absolute paths and parent traversal. On Unix, a backslash is a literal filename character.

The compatibility scanner preserves UTF-8 names, regular files, executable bits and symlink targets. Special files and non-UTF-8 names are refused instead of silently omitted. Portable collision checks and capture-race qualification remain pending.

Before reusing an existing blob, snapshot creation verifies its contents against the captured bytes and refuses a mismatch without advancing the change. This detects preexisting corruption. Command metadata and working-file publication now use the recovery journal described below; concurrent hostile filesystem changes remain outside its qualified guarantees.

## Policy changes in merges and diffs

File equality includes policy metadata and recorded byte length as well as the content hash. A policy-only change appears as modified and survives integration. Concurrent content and policy edits to the same path produce a conflict instead of silently discarding the restriction. Changes to hidden policies contribute only to the restricted-file count, without disclosing paths or domains.

## Repository validation

The CLI accepts only repository format 2 and refuses missing, malformed, older, or newer configuration before running commands. There is no automatic migration. Object IDs must use the expected object-kind prefix and a lowercase hexadecimal suffix. Stored object identities must match the requested identity.

Actor and line keys use nonempty slash-separated components. Control characters, platform path syntax, trailing dots/spaces, and reserved Windows device names are rejected; valid Unicode names remain supported. Encoded keys are limited to 200 bytes. The legacy filename encoding is retained, but an alias such as `team__alice` cannot read or replace `team/alice`. Existing nonconforming actor or line names must be repaired explicitly; this is not an authentication mechanism.

## Identifier compatibility

New repository, change, snapshot, conflict, and operation identifiers retain the full 32 hexadecimal characters of their UUID v4 suffix. Earlier 12-character identifiers remain readable without rewriting history. CLI output may therefore contain longer IDs; the prefix still identifies the object kind.

### Resolving recorded conflicts

After an integration reports conflicts, inspect `rgit conflict list` and
`rgit conflict show <id>`, then record a decision with
`rgit conflict resolve <id> --take incoming` (or `line`, `base`, `delete`).
Use `--as <actor>` consistently for restricted sources. Run `rgit merge preview`
and `rgit line integrate` to publish the resolved merge. Resolution alone does
not change the line or working files.

A decision applies only to its recorded base, line head, and incoming snapshot.
Changing those sources requires a new integration and resolution. Every source
must be visible to the resolver. If the file sides have different access policies,
selected content conservatively becomes admin-only; choosing public content does
not silently remove a concurrent restriction. Resolution operation records are
admin-only. This remains the prototype actor model, not authenticated identity.
Custom merged content is captured with `--from-working` as described below.
Automatic external merge-tool invocation is not yet supported.

### Switching and restoring working files

`rgit workspace switch <change-id>` checks out an existing change's current
snapshot (or its base). It refuses local modifications to tracked files and never
overwrites an untracked collision. Unrelated untracked files remain in place.

`rgit workspace restore --discard-changes` restores the current snapshot's tracked
files, explicitly discarding local edits. Add `--from <snapshot-id>` to restore
another snapshot into the current change; take a new snapshot to record the result.
Without `--discard-changes`, restore refuses dirty tracked files. Both commands
require access to all source metadata and files; they do not produce partial views.

Working-file updates and the workspace pointer share a durable recovery journal.
The next command finishes interrupted publication before reading repository state.
If files were edited after an interruption, recovery stops and retains the journal
instead of overwriting the new edits. Preserve the whole repository and those edits
before manually reconciling that state. Current journal schema 5 upgrades older
schemas under the command lock; older transaction clients refuse the newer schema.

Snapshots record file bytes, access policies, the executable bit and symlink type.
Unix snapshots detect mode-only edits; checkout restores the executable bit while
preserving existing read/write permissions. Recreated files use the process umask.
Legacy records without the bit remain non-executable and keep their manifest hash.
Windows snapshots retain logical modes from the materialized snapshot because
the filesystem does not expose Unix execute bits. Symlinks use the target-text
fallback described below. File/directory transitions use the journal recovery described below. Remaining limitations still prevent a claim of full
Git checkout compatibility. Windows power-loss durability remains unqualified.

Executable-aware recovery introduced journal schema 3, including expected and
target execute bits so mode-only updates recover with file contents. Current schema
4 additionally records symlink type, as described below.

### Checking repository integrity

Run `rgit repo verify --as admin` to validate format-2 record identities, references,
snapshot manifest hashes, blob digests/lengths, parent cycles, conflict state, and
operation links. It reports valid unreferenced blobs without deleting them and
returns a nonzero exit on corruption. Verification does not repair saved records.
Opening the repository still performs normal committed-journal recovery first.
The command requires the admin view because findings concern the complete repository;
this is not a replacement for authenticated identity or signature verification.

### Backing up and recovering saved history

`rgit repo backup /outside/path/new-backup --as admin` verifies the source, copies
saved records and blobs while holding the command lock, and verifies the copy before
publishing its `.rgit` directory. The destination must be new and outside the source
working tree. It gets an independent empty transaction journal. On Unix its root is
private (0700). Existing destinations are never overwritten.

This is a saved-history backup: unsnapshotted edits and untracked files are not
included. To recover, enter the new backup directory, run `rgit repo verify --as admin`,
then `rgit workspace restore --discard-changes --as admin` to materialize its current
snapshot. An interrupted backup without a published `.rgit` is incomplete; keep the
source and retry into a new destination. Backups remain plaintext and need the same
storage protection as the repository. Windows power-loss qualification is pending.

### Merge ancestry

New snapshots record their change base as the first parent. Integrations retain
both the previous line head and the incoming snapshot. Subsequent integrations use
the most recent shared ancestor, so continuing an already-integrated change does
not produce a false conflict against its original base. Reintegrating an ancestor
already present in the line is a no-op. Verification checks all parent edges for
missing records, duplicates and cycles.

Older snapshots omitted some ancestry edges; their declared change base remains a
compatibility fallback when no shared ancestor can be found. Multiple best merge
bases (criss-cross history) are refused pending recursive merge support.

### Exporting native history to Git

`rgit git export /outside/path/new.git --line main --author 'Name <email>' --as admin`
creates a new bare Git repository containing the selected line's saved ancestry,
messages, file bytes, symlink targets, executable modes and merge parents. Git must be installed.
Native snapshots capture the configured repository identity. Legacy snapshots without
recorded authors require the explicit export identity; untouched imported commits
retain their original identities. The result is checked with `git fsck --full --strict` and can
be cloned with Git after export succeeds.

Restricted history is refused unless `--allow-restricted` explicitly authorizes
export without rgit access policies. Git has no equivalent per-object domain model.
The exporter never overwrites an existing destination and removes ambient `GIT_*`
repository variables so they cannot redirect writes. An interrupted export bearing
`RGIT_EXPORT_INCOMPLETE` must not be used; preserve the source and retry elsewhere.

Export writes verified Git objects through Git's [hash-object interface](https://git-scm.com/docs/git-hash-object).
This preserves raw imported commit metadata rather than regenerating signatures or
author headers. Tags, additional refs and submodules still require support.


### Importing Git history

`rgit git import /local/git/repository --revision main --into main --as admin`
imports the selected revision's complete commit ancestry into an empty native line.
Imports default to the admin domain; add `--domain public` only when that is the
intended visibility. The source is checked with Git before import, and native
metadata publishes as one command transaction. Working files remain unchanged until
you switch or explicitly restore. Unsupported trees fail without advancing native
references; failed imports can leave verified unreferenced blobs.

For supported regular-file and symlink histories, untouched commits round-trip with their exact
Git IDs, raw metadata, authors, timestamps, signatures, modes and parent order. SHA-1
and SHA-256 repositories are supported; an export cannot mix object formats. Native
edits can extend imported history, with `identity set` capturing authors on new native snapshots and `--author` supplying
a fallback for legacy snapshots without recorded authors. `repo verify` checks imported commit digests and correspondence between
raw metadata, native files, parents, display messages and timestamps. Signature bytes
are preserved, but trust in signing keys remains Git's responsibility.

Import currently requires a local repository and an empty target line. Tags and
additional named refs are not imported. Submodules, non-UTF-8 names, empty
Git tree entries, unsupported reserved paths and excessive nesting are refused,
not silently rewritten. This is a bounded compatibility path, not complete Git
repository migration or native authenticated synchronization.

### Ignore rules

Snapshots and workspace diffs honor repository `.gitignore` files, including nested
rules and negation. Already-tracked files remain tracked when a new ignore rule
matches them; ignored parents are traversed only to retain those tracked files, and
untracked siblings stay excluded. Missing tracked files are still recorded as
removals. Malformed, unreadable or symlinked ignore files fail capture rather than
silently changing what is saved.

Rules use the [ignore crate's Git matcher](https://docs.rs/ignore/0.4.25/ignore/gitignore/struct.Gitignore.html).
Machine-global excludes and `.git/info/exclude` are not yet loaded. Repository
metadata directories (`.git` and `.rgit`, including case aliases) are always protected.
Other names, including `target` and `node_modules`, follow ignore rules; there are no
implicit build-directory exclusions. Add project rules before capturing generated
files. This is not complete Git ignore configuration parity.

### Git-backed collaboration

`rgit git fetch REMOTE --branch main --into remote/main --as admin` imports a
selected remote branch into a tracking line. Use `--domain public` explicitly for
public history; otherwise newly imported records are admin-only. The first fetch
can target an empty `main`. Subsequent fetches advance only when the remote contains
the previous tracking head; rewinds require a new line for inspection. Fetch does
not overwrite working files or merge local work automatically.

`rgit git push REMOTE --line main --branch main --author 'Name <email>' --as admin`
publishes saved ancestry using Git's ordinary fast-forward checks. There is no force
option. Restricted history requires `--allow-restricted`, which explicitly exports
plaintext without native policies. HTTPS and SSH use the installed Git client's
credential helpers, SSH agent and host verification. Local paths are also supported;
relative paths resolve from the calling directory. Plain HTTP, embedded URL passwords
and external remote helpers are refused. Transport runs with empty client hooks and
TLS verification enabled. Server-side hooks and authorization remain the server's
responsibility.

After a rejected concurrent push, fetch into a separate tracking line, switch to its
head change, use `change retarget main --as admin`, and integrate it with local main.
Resolve any conflicts before retrying the push. Integration updates the saved line;
restore its resulting snapshot explicitly before editing the merged working tree.

The first export of a native snapshot records its Git identity transactionally and
adds its native UUID to the commit header. Later exports preserve that identity;
`--author` applies only to previously unbound native snapshots without recorded authors. A fetch can reconcile
an acknowledged remote push whose local identity publication was interrupted, but
only when the UUID, content tree, parents, message and timestamp match. Remote
publication and local publication are separate transactions: after an ambiguous
push failure, fetch and inspect before retrying. Existing imported signatures and
Git IDs remain unchanged.

Tests exercise two independent clients, concurrent rejected pushes, a combined merge,
tracking rewind refusal, repeated fetch without duplicate ancestry, and interrupted
identity publication against real local Git repositories. Network credential-provider
and server deployments still require qualification. This transport delegates
repository authentication to Git; it does not authenticate `--as`, encrypt the local
compatibility store, implement native per-object authorization, or replace Git itself.
Each fetch currently uses a fresh temporary bare clone; transfer resumption, named
remote configuration, tags, all-ref synchronization and native remote services remain
open.


### Symbolic links

Snapshots store a symlink's target bytes and logical type, never its referent. On
Unix, restore creates real links, including dangling links and links pointing outside
the repository, without reading or modifying those targets. Symlink parents are
still refused during materialization. A regular file and a link containing identical
target bytes remain different entries for diff, merge and dirty-worktree checks.
Git import/export preserve mode `120000` and target blobs in both object formats.

On Windows, imported links materialize as regular files containing the target text,
without requiring link-creation privileges. Subsequent snapshots retain the logical
link type from the current saved baseline. This fallback does not provide executable
OS links or an explicit Windows link-to-regular conversion command. Unix target bytes
may be non-UTF-8; tracked path names must still be UTF-8. Empty/NUL-containing targets,
executable symlink metadata and symlinked ignore-rule files are refused.

Command journal schema 4 records old/new link type with content and executable mode.
Recovery accepts only the old or new entry state; a later type change stops recovery
even when bytes match. Older journals migrate their regular-file records. Older
binaries refuse the new journal version, so preserve a verified backup before
upgrading. Tests include subprocess interruption after link replacement, later-edit
refusal, referent preservation, and Git round trips. Same-principal filesystem races,
and platform durability qualification remain open.

### Custom conflict resolution

Edit the conflicted path, then run
`rgit conflict resolve CONFLICT_ID --from-working --as ACTOR` to save the merged
content. This captures a verified blob immediately; subsequent working edits do not
change the recorded decision. `line integrate` applies the saved resolution only to
the exact recorded source snapshots, and rechecks source access and blob integrity.
The working tree stays untouched until an explicit restore or switch.

Custom resolutions preserve executable/link type and conservatively retain source
restrictions. Use `--take delete` for deletions; a missing working file is not assumed
to be an intentional deletion. `--take` and `--from-working` are mutually exclusive.
Custom blobs are included in repository verification and backups even before they
are integrated. There is no automatic text-merging engine or conflict-marker writer
yet; edit the file with your editor or merge tool before capturing it.

### Named lines and guarded history reset

`rgit line create release --from main --as admin` creates a separate saved line at
main's head, retaining source restrictions. It refuses existing names. New changes
can target that line with `change new NAME --target release`.

`rgit line reset main --to PREVIOUS_SNAPSHOT --expected-head CURRENT_SNAPSHOT --as admin`
moves only the saved line pointer. It preserves working files, changes, snapshots and
blobs. The expected-head guard rejects a stale reset after another writer advances
the line. The operation records both heads; resetting back with the reverse pair can
redo the move. Use explicit workspace restore to materialize a selected snapshot.
A reset does not rewrite history on a remote: Git push still rejects non-fast-forward
publication. This is explicit saved-head recovery, not a complete operation undo/redo
engine or garbage collection.


Logical workspace modes are now anchored to the last restored/captured snapshot,
independently of the current change's ancestry. Restoring another snapshot or starting
a new change preserves those modes when capturing on Windows. Mode-only unsaved
changes also prevent a non-discarding switch, even when file bytes are identical.
The optional workspace `mode_snapshot` reference is journaled with checkout/snapshot
updates and checked by `repo verify`; legacy workspaces fall back to their saved
change baseline. Unix continues to read actual filesystem modes.

### Cloning a Git branch into a native working directory

`rgit git clone REMOTE NEW_DIRECTORY --branch main --domain public` works outside
an existing native repository. It creates a private destination on Unix, fetches the
selected branch into native `main`, verifies history and materializes its saved files.
Omit `--domain public` to retain the admin-only import default. Authentication and
protocol restrictions are the same as `git fetch`.

An existing destination is refused. A transport/import/checkout failure retains the
new directory with `.rgit/clone-request.json`; rerun the same command with `--resume`
after fixing the cause. The remote, branch and domains must match. Resume replays any
committed journal first, then compares checkout against the pre-fetch workspace so
later edits and newly created colliding files are preserved. Move conflicting files
aside intentionally before retrying. Clone resume requires completed initialization
and its recorded clone request. If interrupted before that request exists, finish
initialization with `rgit init --resume` in the destination, then use `git fetch` and
workspace commands to finish manually. This is selected-branch cloning, not all-ref
migration or resumable network transfer.


### Recording native authors

Run `rgit identity set "Name" "email@example.com"` before creating snapshots.
Each new snapshot and integration captures that configured author at creation time;
changing the configuration affects only future snapshots. `identity show` displays
the current setting, and snapshot summaries display recorded/imported author metadata.
Git export uses each recorded author instead of assigning one author to the entire
native history. Its `--author` option remains an explicit fallback for older snapshots
that never recorded an identity. Merely setting an identity does not rewrite those
older snapshots or existing bound Git commits.

Repository verification validates recorded identities and checks that bound Git author
metadata agrees. Native authors are configured metadata, not authenticated principals
or signatures. Git transport credentials remain separate; imported signature bytes
are preserved as before. Configure the identity again when a restored backup changes
owners, since backups retain repository configuration.


### Recovering interrupted initialization

If `rgit init` is interrupted, run `rgit init --resume` in the same directory.
Initialization records its repository identity before staging defaults, and replays
committed command writes before deciding whether anything remains to initialize.
Retrying a completed initialization verifies it without replacing history or adding
another initialization operation. Existing working files are preserved.

Resume refuses unknown control entries, symlinked control directories, incompatible
formats, and saved history whose configuration is missing. It is not a repair command
for a damaged repository. Ordinary repository discovery checks the configuration
format before opening writable command databases, and a malformed nested `.rgit`
stops discovery instead of falling back to a parent repository.

Tests terminate real initialization subprocesses after directory creation, identity
publication, staging, and commit. The command journal has separate publication crash
tests. These cover process interruption; they do not qualify every filesystem or
hardware power-loss behavior.


### Structured status

`rgit status --json` exposes versioned, permission-filtered status for automation.
See [the JSON contract](automation.md) for fields, empty/restricted states and exit
semantics. Default text output remains unchanged.


### Checkout filename aliases

Checkout compares every source/target path prefix using full non-Turkic Unicode
case folding (the pinned Unicode 9 table) followed by NFC normalization. Distinct
spellings with the same key are refused before committing working updates, including
`README`/`readme`, `Src/a`/`src/b`, and canonically equivalent Unicode spellings.
Existing filesystem entries, including untracked directory aliases, are checked too.
Commit and recovery revalidate the working update set before publication.

This is a conservative portable rule on every host. Case-only switches/renames are
currently refused rather than journaled as a special rename operation. Saved Git
history with colliding paths remains importable, verifiable and exportable; checkout
refuses to materialize it. This check does not qualify hostile concurrent filesystem
changes or every platform-specific filename alias such as Windows short names.


### Reviewing changed text

Add `--patch` to `diff workspace`, `diff snapshot OLD NEW`, or `diff line NAME` for
unified text hunks with three context lines. This is implemented in Rust and does
not invoke Git or external diff/textconv tools. Default filename summaries remain
unchanged. Headers quote unusual path bytes; executable/symlink mode changes are
shown, and symlink previews use target text without following referents.

The preview applies the same file visibility rules as summary diffs. Hidden changed
paths contribute only a count. Saved text is read through validated blob paths and
checked against its recorded digest/length; working bytes are rechecked against the
scan. A read or verification failure returns nonzero, potentially after earlier
files were printed.

This is a review preview, not complete interchange: binary/non-UTF-8/control-bearing
content and files over 1 MiB are labeled as omitted. Access-policy changes are noted
but cannot be encoded in Git text hunks. Use Git export for complete supported saved
history interchange. Ordinary unrestricted text hunks are tested by applying them
with Git, including additions, deletions, modes and missing final newlines.

Each file's diff uses a 500 ms algorithm deadline; on difficult inputs the library
may produce a less minimal valid diff. This is not a hard command runtime limit:
scanning, reading and rendering still take time. See the pinned library's
[deadline semantics](https://docs.rs/similar/2.7.0/similar/struct.TextDiffConfig.html#method.timeout).

### Merge tree validation

Merge preview and integration validate a clean result's manifest and selected blobs
before reporting success or publishing a snapshot. Independently added paths such as
`a` and `a/b` must not become files in the same snapshot. File/directory collisions become one explicit conflict at the shared root.
Integration saves the conflict without advancing the line. This also
checks the result after saved per-path conflict decisions have been applied.

### Resolving structural merge conflicts

A `file_directory` conflict groups the root and every affected descendant. Resolve it
with `conflict resolve ID --take base|line|incoming|delete`, then integrate again.
Choosing a side selects its complete subtree; `delete` removes the entire group.
Unrelated paths, including names that only share a text prefix, merge normally.
`--from-working` is refused for a structural conflict; for custom content, edit and
snapshot the incoming change, integrate again, and choose the updated side.

All source subtree policies are checked, and mixed source policies conservatively
restrict selected files to admin. Decisions remain bound to exact source snapshots;
a per-file decision cannot be reused as a subtree decision. Older clients cannot
read the new conflict kind, so keep a verified backup before upgrading.
### Recoverable file/directory checkout transitions

Checkout can replace a tracked file or symlink with tracked descendants, and replace
tracked descendants with a file or symlink. It validates the complete old/new path
sets before committing the journal, removes tracked leaves before creating targets,
and removes only empty directories. Untracked leaves and nested `.git`/`.rgit`
directories block replacement, including with `--discard-changes`.

Journal schema 5 preserves these transition semantics across interruption; older
clients refuse this version. Recovery checks all affected entries before publishing,
accepts intermediate directory states, and preserves later untracked work unless it
collides with a target, in which case recovery stops for manual reconciliation.
Subprocess tests terminate after six filesystem publication points in each direction.
Case-only and Unicode-equivalent spelling transitions remain conservatively refused.
These process interruption tests do not qualify power-loss or hostile filesystem races.
