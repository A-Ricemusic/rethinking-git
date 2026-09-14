# Agent/Git collaboration trial

A disposable React/Vite application was evaluated on macOS arm64 on September 14,
2026 using a release build. The starting Git-upstream implementation was based on
`43a6ca7f369608c3ca8b864c6100e937d84dfdf6` plus the candidate changes; it was not a
published release. The application and raw command logs were outside this checkout.

## Findings

- Git import preserved the original commit ID. Saved upstreams, native worktrees,
  Git-peer contributions, guarded dirty pulls, divergent pushes, explicit conflict
  resolution, server rejection and verified backups worked in the trial.
- A clean file status could hide saved but unintegrated work. Agents needed extra
  commands and ancestry reasoning to know whether their change would be published.
- A second native worktree could integrate after another agent previewed a push.
  An unguarded push then included the newer line head. This is existing shared-line
  behavior, but unsuitable for automation that assumes it is publishing only what
  it reviewed.
- Listing every recovery snapshot made push previews increasingly verbose.

## Changes and regression evidence

`status --workflow` adds an opt-in, offline assessment of saved-work integration,
materialization versus the target line, and advisory next actions. Permission-limited
assessments report restricted/unknown state. Ordinary status avoids ancestry traversal.

`push --expect-snapshot` compares the selected line head under the shared command
lock before export/transport. A mismatch returns the typed `stale_snapshot` error.
An agent should also use the explicit destination from its preview to avoid a
concurrent upstream-setting change. Remote authorization and fast-forward checks
remain separate.

`push --dry-run --max-commits N` bounds displayed commit details while retaining
exact ahead/behind counts and explicit truncation metadata. Default N is 50; N=0
provides a compact summary. This limits output, not transfer or history traversal.

Executable regressions in `tests/git_remotes.rs` reproduce the shared-worktree race,
assert unchanged local/remote records on refusal, retry the approved head, and check
bounded output. `tests/json_api.rs` covers clean/unintegrated work, integration,
divergence, materialization lag and restricted metadata. The longer disposable trial
replays the improvements with 55 local recovery checkpoints and a real Git peer.

## Assessment

Saved remotes and explicit publication guards make rgit easier to automate. The
workflow summary reduces ambiguity between saved files and integrated history.
These improvements do not establish production Git parity: intermediate snapshots
still become exported commits, previews rebuild scratch Git history, and network
credentials, large repositories and physical storage failures need broader testing.
The review is an automated engineering evaluation, not independent release approval.
