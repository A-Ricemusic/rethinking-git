# Working locally in rgit with a Git remote

The Git bridge imports a selected branch into native storage and publishes saved
line history as ordinary Git commits. Git users can keep collaborating through the
same remote. Installed Git supplies HTTPS/SSH transport and credentials.

## Clone and publish

```sh
rgit git clone git@github.com:OWNER/PROJECT.git project-rgit \
  --branch main --domain public
cd project-rgit
rgit identity set "Your Name" "you@example.com"
rgit workspace start improve-search --target main
# Edit files, run tests, and capture the change.
rgit snapshot --message "Improve search"
rgit line integrate main --as admin

# Publish to a feature branch for review.
rgit upstream set origin improve-search --line main --as admin
rgit push --dry-run --as admin
rgit push --as admin
```

Clone saves `origin` and associates native `main` with the selected Git branch,
including when the remote branch is not called `main`. The destination must be new;
cloning a local Git directory imports committed history, not its uncommitted files.
`--domain public` selects local visibility, not GitHub repository visibility.
`--as admin` selects the local full-access view; it is not Git authentication.

Push publishes the selected line's saved ancestry. Unsaved edits and snapshots not
integrated into that line are excluded. Intermediate snapshots and integration
commits remain part of exported ancestry; publishing one commit per logical change
is not implemented. Set your author identity before taking snapshots.

## Configure an existing native repository

```sh
rgit remote set origin git@github.com:OWNER/PROJECT.git --as admin
rgit upstream set origin main --line main --as admin
rgit remote list --as admin
rgit upstream show --as admin
rgit pull --as admin
```

`remote set` adds or replaces a destination. Local paths are resolved to absolute
paths when saved, so commands work from subdirectories and linked worktrees.
Remote and upstream settings are shared across native worktrees and retained in
verified backups. They live in `repo.json`, are journaled with command writes, and
contain no Git credentials. Older binaries do not understand these added settings
and may drop them when rewriting repository configuration; use an updated binary
across worktrees.

With no remote argument, push and pull use the current change's target line (or
`main` when there is no current change) and require a configured upstream. `--line`
selects another line. `--branch` overrides the destination branch for one command
without changing saved tracking.

For compatibility, supplying an explicit remote name, URL, or path retains the old
`main` line default unless `--line` is given. The branch comes from that line's
upstream when the supplied remote name matches; otherwise it defaults to `main`.
Explicit `--branch` always wins. Names take precedence over bare relative paths;
use `./origin` to refer to a directory when a saved remote is also named `origin`.

The existing `rgit git push` and `rgit git pull` spellings remain supported and
accept the same options as `rgit push` and `rgit pull`. Fetch accepts named remotes:

```sh
rgit git fetch origin --branch main --into upstream-main --as admin
```

Remove tracking before removing a remote that is in use:

```sh
rgit upstream unset --line main --as admin
rgit remote remove origin --as admin
```

## Preview before publishing

```sh
rgit push --dry-run --as admin
rgit --output json push --dry-run --as admin
```

Preview exports into temporary storage and contacts the remote. It reports:

| State | Meaning |
| --- | --- |
| `new_branch` | The destination branch does not exist. |
| `up_to_date` | Both tips are the same Git commit. |
| `fast_forward` | Only local commits are missing from the remote. |
| `behind` | Only remote commits are missing locally. |
| `diverged` | Both sides have commits absent from the other. |

The `git_push_preview` JSON record includes the line, resolved remote, branch,
local and remote commit IDs, local snapshot ID, ahead/behind commit counts, and
outgoing commit IDs/subjects. `fast_forward_allowed` describes ancestry only;
server authorization and branch-protection rules can still reject publication.
A successful preview exits zero even for `behind` or `diverged`; inspect `state`.
Transport or validation errors return nonzero with the usual JSON error envelope.

Preview does not bind native snapshots to Git identities, change saved native
records or working files, or publish remote references. As with other commands,
repository opening first recovers any previously committed pending transaction.
Preview is a point-in-time comparison, not a reservation: push rechecks remote
fast-forward rules, and the remote can change between preview and publication.
It currently exports local ancestry and fetches remote history into scratch storage,
so it is not yet an incremental large-repository status operation.

A clean fast-forward `pull` materializes incoming files and starts a change at the
new tip. Dirty or unintegrated work and divergence retain the existing guarded
refusal behavior. Fetch divergent work into a separate line and resolve it using
the [explicit integration workflow](getting-started.md#collaborate-through-a-git-remote).
Ordinary `status` remains a local file comparison; use preview for live upstream
counts. Remote tracking does not automatically follow a feature branch back to
`main` after a pull request is merged.

Selected-branch compatibility limits still apply: this is not all-ref/tag migration,
submodule support, native authenticated synchronization, or a full backup of native
workspace and operation state on GitHub.

## Agent workflow checks

`rgit status --workflow --json --as admin` adds an offline `workflow` object to
status. It separates dirty working files from saved work awaiting integration:

- `saved_work`: `no_change`, `none`, `unintegrated`, `integrated`, `diverged`, or
  `restricted`. Integration is determined by ancestry, not file equality.
- `workspace_has_changes`: the existing file comparison, including hidden changes;
  `null` when there is no active change and no comparison was made.
- `materialized_matches_line`: whether the last materialized/captured tree equals
  the current target line's tree, including modes and policies. This does not
  compare live unsaved edits. `null` means unavailable or restricted.
- `next_actions`: advisory action names, not commands to execute automatically.
  `snapshot`, `integrate`, `review_integration`, and `inspect_line` identify the
  pending decisions; `inspect_permissions` means the view cannot assess history.
  `preview_push` suggests reviewing publication, not that the remote is current or
  that publication is authorized. `edit` and `start_change` describe local setup.

The target line name is omitted when restricted. This optional assessment traverses
local ancestry; ordinary status keeps its existing cost and output. It does not
contact Git or a remote and does not resolve conflicts or replace working files.

### Keep previews small

Preview shows at most 50 outgoing commits by default. Use `--max-commits N` (0–1000)
with `--dry-run` to choose a limit; zero returns counts without commit details.
It shows the newest N outgoing commits in oldest-to-newest order within that window.
`commits_total` and `ahead` always count all outgoing commits;
`commits_truncated` reports omitted entries. Limiting output does not limit the
history that a subsequent push publishes or the history exported for comparison.

### Publish only the reviewed local snapshot

Agents sharing native worktrees also share line heads. Another agent can integrate
work between your preview and push. Pass the preview's `snapshot_id` back explicitly:

```sh
rgit --output json push --dry-run --max-commits 20 --as admin
# Review the result, then use its line, branch, remote, and snapshot_id:
rgit push REMOTE --line LINE --branch BRANCH --expect-snapshot SNAPSHOT_ID --as admin
```

The guard is checked under the shared command lock before exporting or contacting
the remote. A changed line returns nonzero with JSON `error.kind: "stale_snapshot"`
and no published outcome; preview again and review the new work. Retrying the same
snapshot is allowed and retains ordinary fast-forward checks. Supply the explicit
remote, line and branch from the preview to also pin the destination if another
agent can change upstream settings. The guard compares the current snapshot ID,
not whether the line ever moved away and back; it does not reserve remote state or
replace server authorization. Push without the guard retains its existing behavior.
