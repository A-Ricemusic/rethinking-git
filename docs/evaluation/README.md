# Native development checkout evaluation

`scripts/evaluate-self-host.py` exercises the current repository as a native working
copy. It requires a built `rgit` with JSON status, installed Git and Cargo, and a new
evaluation destination outside the source checkout:

```sh
cargo build --release --locked
python3 scripts/evaluate-self-host.py target/release/rgit . ../native-evaluation \
  --source-revision FULL_SOURCE_COMMIT
```

The selected local Git branch (default `master`, configurable with `--branch`) must
match the supplied full source commit. Build the binary from that clean revision;
the report records its SHA-256 and the supplied provenance, not an independent build
attestation. The source checkout and its remote are not modified. The evaluation
directory is retained, including phase stdout/stderr logs and successful `report.json`.
If a phase fails, inspect its logs; rerun in a new destination after fixing the cause.

The workflow checks:

- Native clone and verification with no `.git` working repository.
- Exact unchanged Git commit identity after export and `git fsck --strict`.
- Full workspace tests and a release build inside the native checkout.
- Clean status after building, including the executable built in that checkout.
- Native edit, author capture, snapshot, integration and Git export of the changed bytes.
- Verified saved-history backup and materialization of the backup.
- Guarded line reset, unchanged working files until explicit restore, and final verification.

Reported test pass counts sum printed test-result summaries, including any subprocess
harness summaries; they are not a count of unique test functions. Local evaluation is
not a speed benchmark, network-authentication test, power-loss qualification or
independent release/security review. It complements the smaller CI scenarios and does
not close the remaining blockers in the [readiness audit](../production-readiness.md).

## Recorded run

[The September 12, 2026 receipt](native-self-host-2026-09-12.json) records a successful
macOS arm64 evaluation of source commit `6a4ae7e5c3b3c60c8c81cc99e63971607ce48c7c`
(the combined candidate subsequently merged in PR #34). Its 132 commits and 133
tracked files cloned into native storage; unchanged export reproduced that exact
Git tip. All workflow phases completed, including the full workspace test command,
release build, changed-content export, backup restore and guarded reset/restore.
The final repository verified with 134 snapshots and no unreferenced blobs.

## Disposable React app trial

The [React development experience report](react-vcs-trial.md) exercises all current
command paths against a temporary React app, records workflow limitations and an
agent-adoption recommendation, and confirms removal of the app and its histories.
Its [receipt](react-vcs-trial-2026-09-12.json) preserves command outcomes and cleanup
metadata without retaining the app.

[Workflow improvements](workflow-improvements.md) records the implemented fixes and
regression evidence following the React trial, including structured outcomes and
native linked worktrees.
