# Working CLI guide

This guide describes the current executable. Commands below use a POSIX shell;
PowerShell users can run the same `rgit` commands and create/edit files with their
usual tools. Rust 1.85 or newer builds the CLI. Installed Git is required only for
Git interoperability and transport.

Build/install from the source checkout with `cargo install --path . --locked`, or
build without installing with `cargo build --release --locked` and use the binary
at `target/release/rgit` (`rgit.exe` on Windows). `rgit --help` and subcommand
`--help` show the actual command surface.

## Save and integrate a change

Start in a fresh evaluation directory:

```sh
mkdir demo
cd demo
rgit init
rgit identity set "Example Developer" "dev@example.com"
printf 'hello\n' > README.md
rgit workspace start first-change
rgit status
rgit diff workspace --patch
rgit snapshot --message "Add README"
rgit line integrate main --as admin
rgit repo verify --as admin
```

A change is a stable unit of work. Each `snapshot` captures its files and advances
that change's saved tip. `line integrate` merges the saved tip into the selected
line; it does not capture unsaved edits or automatically replace working files.
Use `diff workspace --patch` to review text before capture, and `status --json`
for the [versioned automation interface](automation.md). Binary/large content is
explicitly omitted from text previews; [the reference](prototype.md#reviewing-changed-text)
describes those limits. The author is recorded when a snapshot is created. Configuring another author later
does not rewrite previous snapshots.

`--as admin` selects the built-in local view with all domains. It is **not** a login,
credential, signature, or protection against someone who can read `.rgit` directly.
Do not use actor filtering as a secret-storage boundary.

The scanner respects root/nested `.gitignore` rules while retaining previously
tracked files. It excludes `.git` and `.rgit` control paths. Regular files,
executable flags and symlink targets are represented; unsupported names and file
types fail instead of being silently omitted. On Windows, symlinks materialize as
regular files containing their target text, with logical type preserved in metadata.

## Switch, restore and resolve

Use `rgit workspace start NAME --target main --as admin` to create a change and
check out its target line in one recoverable transaction. The target line's policy
is inherited. Dirty tracked files, restored-but-unsaved changes and untracked
collisions refuse the operation without creating a change or changing files.

Two baselines serve different purposes. Status/diff compare against the current
change's saved tip (or its inherited line head). Checkout path ownership comes
from the last captured/materialized snapshot. Restoring another snapshot changes
the latter without rewriting change ancestry. Consequently, restored contents may
still appear modified in status; snapshot them to save them on the current change.
Explicit `restore --discard-changes` can move between saved file/directory shapes
without intermediate snapshots. Ordinary switching continues to protect restored
changes. `status --json` exposes both references.


`rgit change list --as admin` prints change IDs. `rgit workspace switch CHANGE_ID
--as admin` materializes an existing change and refuses unsaved tracked changes or
untracked collisions. `change new NAME --target LINE` starts a change at that line's
saved head **without replacing current working files**. Save or intentionally set
aside existing edits before choosing what to materialize.

To discard edits to tracked files and restore the current change's saved baseline:

```sh
rgit workspace restore --discard-changes --as admin
```

To restore another saved snapshot, add `--from SNAPSHOT_ID`. That changes working
contents, not change ancestry; take a new snapshot to record the result. Untracked
colliding files are refused even with `--discard-changes`.

Integration conflicts leave the line unchanged and return a nonzero status. Inspect
`rgit conflict list --as admin` and `rgit conflict show CONFLICT_ID --as admin`.
Resolve each conflict by choosing a side:

```sh
rgit conflict resolve CONFLICT_ID --take incoming --as admin
```

The available choices are `base`, `line`, `incoming`, and `delete`. For a manual
merge, edit the file and use `--from-working` instead of `--take`. This captures
those bytes immediately. Retry `line integrate` after resolving all conflicts.
Decisions are tied to the exact source snapshots; changed inputs require a fresh
resolution. Integration does not insert conflict markers. It automatically merges supported
non-overlapping UTF-8 text edits; overlapping edits, incompatible modes/policies
and unsupported content remain explicit conflicts. Restore the resulting saved line head deliberately when ready to use it.

## Back up and recover saved work

From `demo`, write a verified backup to a new sibling directory:

```sh
rgit repo backup ../backup --as admin
cd ../backup
rgit repo verify --as admin
rgit workspace restore --discard-changes --as admin
```

The backup contains saved history and configuration, including author identity; it
does not contain unsaved working edits. Verification detects corruption and missing
references but does not repair them. Normal startup first replays any committed
command journal. If recovery refuses a later working-file edit, preserve that edit
before reconciling; do not delete transaction databases to make the error disappear.

After interrupted initialization, `rgit init --resume` retries in place while
preserving the recorded repository identity and working files. It refuses to invent
configuration over saved history whose configuration is missing.

## Collaborate through a Git remote

Git transport uses Git's SSH/HTTPS credentials. The examples use a local bare Git
repository so they need no account or network setup. From the original `demo`:

```sh
rgit git export ../remote.git --as admin
cd ..
rgit git clone remote.git colleague --domain public
cd colleague
rgit identity set "Second Developer" "second@example.com"
printf 'hello from colleague\n' > README.md
rgit snapshot --message "Update README"
rgit line integrate main --as admin
rgit git push ../remote.git --as admin
```

`git export` creates a new bare Git destination; it will not overwrite one.
`git push` publishes to an existing remote and refuses non-fast-forward updates.
For a real remote, replace the path with its HTTPS or SSH URL. Restricted native
history requires explicit `--allow-restricted` on export/push because Git does not
carry native access policies.

Back in `demo`, pull a fast-forward update into a clean workspace:

```sh
cd ../demo
rgit git pull ../remote.git --as admin
rgit status
rgit repo verify --as admin
```

Pull materializes the updated line and starts an empty change at that exact saved
snapshot. It inherits the existing line policy. Dirty files, untracked collisions,
unintegrated saved work, and divergent history are refused without changing saved
records or working files. An unchanged remote tip preserves the current workspace.

For divergent work, fetch into a separate tracking line and integrate explicitly:

```sh
rgit git fetch ../remote.git --into upstream --domain public --as admin
rgit change retarget upstream --as admin
rgit line integrate upstream --as admin
```

Resolve any reported conflicts and repeat integration, then push with `--line
upstream`. Integration changes saved history; use an explicit workspace restore from
the resulting line-head snapshot when ready to materialize and test the combined
contents. A stale push must be reconciled through fetch and integration; there is no
implicit force push.

Import/fetch/clone default to the admin domain. `--domain public` above deliberately
makes this demonstration's imported history visible in the default view. Clone
requires a new destination; failed clones with a recorded request can be retried
with the same command plus `--resume`. See the [clone recovery details](prototype.md#cloning-a-git-branch-into-a-native-working-directory).

## Evaluation limits

The Git bridge preserves supported selected-branch history and commit identities,
including imported signatures. It does not migrate every ref, tags, submodules or
non-UTF-8 paths. File/directory checkout transitions are supported with untracked-file protection.
Case-only transitions and ambiguous criss-cross merge bases remain unsupported. Native authenticated synchronization, canonical-store CLI
migration, complete operation undo, large-repository qualification and release
security/durability reviews remain open. The [readiness audit](production-readiness.md)
tracks these gaps; passing this guide is a workflow check, not release approval.

## Concurrent tasks and automation

Use `rgit worktree add ../task --name task --from main` to create a native linked
working directory with its own change. Shared lines and history are immediately
visible from all linked directories. See [worktrees](worktrees.md) for resuming
creation, independent workspace pointers, and detaching while preserving files.

Use `rgit --output json workspace start task` and `rgit --output json snapshot`
when driving the CLI from an agent. Read IDs from typed records instead of parsing
terminal text or opening `.rgit` files; [automation](automation.md) defines errors,
conflict outcomes, schema compatibility, and text truncation.
