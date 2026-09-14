# Native linked worktrees

A native worktree is another working directory attached to the same `.rgit`
history store. Each directory has its own current change and materialized snapshot.
Snapshots, changes, lines, policies, identity, and the operation log are shared.
No Git repository, copied object store, or synchronization step is involved.

From an initialized repository with saved history on `main`:

```sh
rgit worktree add ../agent-a --name agent-a --from main
rgit worktree add ../agent-b --name agent-b --from main
rgit worktree list
cd ../agent-a
# edit files
rgit snapshot --message "Implement task A"
rgit line integrate main
```

The second directory sees the updated line immediately, while its working files
and current change stay unchanged. It can integrate its own saved change normally.
Use a fresh change per agent. Switching to a change already active in another
registered worktree is refused, including when that directory is temporarily
unavailable. `workspace start` creates a fresh change in the current directory;
`worktree add` creates a separate directory and change.

The destination must not exist, its parent must exist, and working directories
must not overlap. Paths are fixed registrations: moving a linked directory or its
primary repository is unsupported. Worktrees depend on the primary repository's
control store; they are not backups. Use `repo backup` for an independent verified
saved-history copy. A backup made from a linked directory keeps that directory's
workspace reference and shared history, but not linked registrations or unsaved
files. Restoring the backup does not depend on the original directories.

## Creation and recovery

Creation first records an intent and registration, then checks out the selected
line snapshot and creates a change in one command transaction. If interrupted or
refused after registration, retain the directory and retry the exact request:

```sh
rgit worktree add ../agent-a --name agent-a --from main --resume
```

Resume uses the originally selected snapshot even if the line has advanced. It
never resets an already initialized workspace or its dirty files. Correct a stated
cause such as missing/corrupt saved data before resuming. An interruption before a
valid intent is published can leave an empty destination/control directory; inspect
it before removing that empty directory and retrying. Arbitrary existing folders
cannot be adopted with `--resume`.

All directories share one process lock and recovery journal. Commands serialize
metadata updates. Journal schema 6 records which workspace owns a working-file
transaction; a command in any directory recovers that transaction in its original
registered directory. The directory and its marker must agree with the registration
before recovery writes there. Missing directories without pending working-file
updates do not block other worktrees. A missing directory needed for recovery does
block commands until restored; recovery never redirects its writes elsewhere.

Opening with this implementation upgrades older command databases to schema 6.
Older binaries refuse the newer databases. Preserve a verified backup before an
upgrade; copying an older executable over the new one is not a rollback strategy.
This extends process-interruption recovery, not qualification of every filesystem
or hardware power-loss scenario.

## Detach without losing working files

```sh
rgit worktree detach ../agent-a
```

Detach removes the link and preserves **all** files, including dirty, untracked,
and ignored files. Saved changes and snapshots stay in shared history, and the
change becomes available to another workspace. Detach is idempotent; rerun it if
interrupted. It removes only its own marker and an empty control directory. Delete
the remaining ordinary folder separately when its contents are no longer needed.
Detached registrations remain visible until explicitly pruned. Relocation and
automatic stale-worktree cleanup are not yet supported.

Management uses the local admin view; `add --as ACTOR` additionally selects the
checkout view. This is not authenticated multi-user isolation. Separate working
directories help agents avoid file collisions, but shared policies and line updates
still require coordination.

All worktree commands support `--output json`; `worktree` records include `id` (null
for the primary directory), `path`, `change_id`, `available`, and `detached`.
Creation also returns the usual `change_created` and `workspace` records when a
new change is created. See [automation](automation.md) for outcome handling.

## Retire a registration and reuse its path

After detaching a worktree, use its ID from `worktree list`:

```sh
rgit worktree prune wt_EXACT_ID_FROM_LIST
```

Prune accepts a detached registration or one whose directory no longer exists.
It refuses an existing active directory, including a damaged marker: detach first.
The shared recovery journal is processed before pruning; pending working-file
publication that needs an unavailable directory blocks the operation. Restore that
directory and complete recovery before retiring it. A moved or temporarily offline
worktree should be restored rather than pruned.

Prune does not delete files, changes, snapshots, or blobs. It retires the active
registration, retains an internal receipt and saved workspace metadata, releases
its change, and allows the old path to be registered again after you remove the
remaining ordinary directory. It is not garbage collection. Retrying the same ID
returns `changed: false`; unknown IDs are refused. `--output json` returns a
`worktree_pruned` record containing `id`, `path`, and `changed`.
