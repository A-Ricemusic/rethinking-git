# React development trial: rgit experience and agent suitability

Evaluated September 12, 2026, on macOS arm64 at source
`d2919690186ecfd2495b0ee43508ddd50c617628`. The
[execution receipt](react-vcs-trial-2026-09-12.json) records binary provenance,
commands, exit codes, assertions, observed surprises, installed versions and cleanup.

**My recommendation: agents should use Git for normal professional work today,
usually with a separate Git worktree per concurrent task.** Rgit is useful for
controlled experiments with stable changes and explicit conflict decisions, but
this trial exposed additional state-management work that an agent must handle.
This is an assessment of the implementation tested here, not a claim that its
underlying model cannot become a better agent interface.

## What I actually did

I scaffolded a disposable React app using the [Vite React template](https://vite.dev/guide/),
then made it a small working counter with a heading and footer imported from a
shared settings module. Installed versions were React/React DOM 19.3.0, Vite 8.3.0,
Oxlint 1.82.0 and the React Vite plugin 6.1.1. The environment used Node 24.16.0,
npm 11.16.0, Rust 1.97.0 and Apple Git 2.50.1.

The app lived under the parent repository's ignored `target/` directory. It had
its own native `.rgit` repository. Source-control mutations used the rgit CLI;
Git was used for its transport bridge and independent `fsck`/commit-ID checks.
I read native JSON records to obtain IDs and assert state, but did not edit those
records to manufacture successful workflows. The intentional corruption probe
changed one blob in a disposable backup and restored its original bytes afterward.

The browser rendered the merged heading, “Resolved React Counter,” and the
independently edited footer. Clicking the counter changed `Count: 0` to `Count: 1`.
Builds and lint passed before collaboration, after conflict resolution, after
synchronization and from a restored backup.

## Coverage and results

The trial executed 233 recorded commands and completed 48 explicit assertions.
All 40 current command paths were exercised, including read/list commands. This
means command-path coverage, **not** every flag combination, failure state, platform
or security property. Four command outcomes differed from the harness's initial
expectations; they are discussed below rather than hidden in a blanket pass claim.

| Area | Observed result |
| --- | --- |
| Initialization and identity | Native init, completed-init resume, author configuration and inspection worked. |
| Snapshots and inspection | Stable change IDs, successive snapshots, status JSON, workspace/snapshot/line text diffs and history inspection worked. |
| Ignore rules | Dependencies, build output and the synthetic `.env` stayed out of snapshots; previously tracked files remained tracked after a new ignore rule. |
| File fidelity | Binary bytes, an executable script and a symlink target survived capture and restoration on macOS. |
| Workspace safety | Dirty switches, genuine untracked collisions and case-alias collisions were refused. Explicit discard restored tracked content. Shape/baseline sequencing caused the limitations described below. |
| Text integration | Independent edits to different parts of the same settings file merged automatically. Integration preserved the existing working files until an explicit restore. |
| Conflicts | Custom working content and all four side choices—base, line, incoming, delete—worked. A file/directory conflict was represented explicitly and resolved to the chosen subtree. |
| Lines | Create, inspect, retarget and guarded reset worked. A stale reset preserved records; a successful reset left working files unchanged. |
| Actor/path policies | Restricted paths and direct reads were filtered. Local self-granting of admin succeeded, confirming that actor selection is not authentication. |
| Git exchange | Export/import/clone worked. Two native peers exchanged independent changes; stale push rejection followed by fetch, retarget, integrate and push reconciled them. Git round-trip commit IDs matched exactly and `git fsck --strict` passed. |
| Pull | A dirty fast-forward pull preserved records and edits. A clean pull materialized remote work. An unchanged tip preserved dirty work and the current change. |
| Export restrictions | Restricted history export required `--allow-restricted`; the acknowledged export succeeded to a disposable local destination. |
| Backup and verification | Restored app sources matched, and the backup independently installed, built and linted. Verification detected a corrupted blob without changing saved records. |
| Concurrent commands | Two simultaneously launched snapshot commands both completed and formed a parent chain without losing either revision. This did not test simultaneous source-file editing. |
| Diagnostics | `--version` and a native `worktree` command were absent, returning syntax errors. |

Final native verification reported **29 changes, 7 lines, 43 snapshots, 6 conflict
records, 159 operations and 37 blobs, with zero unreferenced blobs**. All six
conflicts were resolved and JSON workspace status was clean. These are trial
repository counts, not counts of tests.

Separately, `cargo test --workspace --locked` passed on this machine. That suite
adds existing process-interruption, transaction, interoperability and other
regressions beyond the React scenarios. Its success does not erase the workflow
limitations uncovered here. This trial did not test network credentials, remote
service authorization, interrupted transfer, real power loss, other operating
systems, or large-repository performance.

## Where the workflow became awkward

### Restoring content does not update the change's tracking baseline

I saved a version where `notes/decision.txt` had become a directory containing a
file. Restoring an older snapshot with a plain file at that path succeeded. Trying
to restore the directory version again failed with:

```text
Error: checkout path has an unsafe parent
```

The active change still referred to the directory-shaped saved snapshot, despite
the file-shaped working contents. No destructive overwrite occurred. I could
continue by snapshotting the restored file-shaped state, restoring the directory
snapshot, and snapshotting that result before integration. Those extra snapshots
were a real cost of the current workflow, not part of the intended feature work.

### Creating a change changes context without materializing its base

With the saved directory-shaped version on disk, I created another change based
on the file-shaped main line. Restoring its base then failed with:

```text
Error: checkout would replace a directory containing untracked files
```

The nested file had been saved in the previous change, but was untracked relative
to the new active change. I preserved the carried contents with another snapshot,
then restored the line. For subsequent fresh-change setup, I restored the desired
line while still in the old context **before** creating the new change. This is a
trial workaround, not a general recipe for discarding arbitrary user edits.

These refusals are safer than silently overwriting files. Nevertheless, an agent
must understand three separate states: working contents, the current change's
saved tip and the target line's tip. A successful integration adds another reason
to materialize explicitly before running tests. I would prioritize an atomic
“start a change and check out its base” operation and clearer restoration-baseline
semantics before recommending unattended adoption.

### Two surprises belonged to the harness, not rgit

The generated Oxlint command inherited the outer repository's `target/` ignore
rule and found no files. I scoped it explicitly to `src` and `vite.config.js` with
`--no-ignore`. Subsequent lint runs checked actual source. The scaffold uses
Oxlint, not ESLint.

My first “untracked collision” probe was still tracked by the active change, so
`--discard-changes` correctly allowed replacement. I corrected the setup to use a
change whose baseline omitted that path; the genuine untracked collision was
then refused. The receipt retains both outcomes.

## Pros and cons from an agent workflow perspective

**What worked well:** saving successive attempts under one stable change ID made
the logical task easy to follow. Versioned JSON status was straightforward to
consume. Explicit conflicts supported deliberate decisions without parsing inline
markers. Conservative refusals, guarded line resets, verified backups and exact
Git identity checks made it possible to assert that operations preserved intended
state. The app remained buildable after real merges and synchronization.

**What required extra orchestration:** I needed several commands and read-only JSON
lookups to manage snapshot/change IDs and materialize exact results. Most commands
lack a comparable JSON interface. The baseline surprises required extra snapshots.
The absence of native linked worktrees means separate agents do not yet get a
shared repository with isolated editable directories. Serialized commands protect
metadata, but do not isolate agents editing the same files. Snapshotting the whole
working tree also requires careful ignore rules and does not provide selective
staging. The operation log is useful inspection, but is not a complete undo system.

There is also documentation drift: the older prototype introduction says remotes
are absent, the working guide says automatic text merging is absent, and its limits
still list file/directory checkout as unsupported. The trial demonstrated those
capabilities while also finding their remaining constraints. Agents following
those statements could take unnecessary detours. No implementation or guide text
was silently changed during this evaluation to improve its results.

Finally, the CLI remains the compatibility JSON/journal implementation rather than
the canonical store path. Actor filtering is not a security boundary, and Git
interoperability is a supported subset rather than complete Git parity.

## Should other agents adopt it?

For routine repository work, I would choose Git with explicit branch/worktree
ownership and scripted safety checks. Git already provides
[linked worktrees](https://git-scm.com/docs/git-worktree) and
[stable porcelain status output](https://git-scm.com/docs/git-status), including
NUL-delimited paths for automation. The useful comparison is against an agent
using those facilities competently, rather than against an agent guessing at
human-oriented Git output.

Rgit could benefit agents that want stable task identities and structured conflict
decisions, particularly in disposable evaluation repositories. I would first want
atomic workspace transitions, consistent machine-readable mutation results,
native worktree isolation, reliable operation undo, accurate guides and the
remaining storage/security qualification. This trial demonstrated a functioning
small-app workflow; it did not demonstrate superior agent productivity or justify
replacing Git in a professional repository.

## Cleanup and retained artifacts

The complete temporary sandbox was removed, including the React app, dependencies,
build output, native histories, collaborator checkouts, backup and local bare Git
remotes. The dev-server process group was stopped and its port checked closed;
the browser was navigated away. The parent Git repository tracked zero files from
the sandbox, no root `.rgit` was created, and no app history was pushed to GitHub.

Only this report and its JSON receipt are retained from the experiment. The receipt
contains command/result metadata and hashes, not the app or its snapshots. The
ad hoc harness, which embedded app source, and temporary logs are removed during
cleanup. Their hashes are provenance notes, not independently replayable evidence
or an attestation. The source revision and workflow descriptions provide the scope
for repeating an equivalent trial without retaining the requested disposable app.
