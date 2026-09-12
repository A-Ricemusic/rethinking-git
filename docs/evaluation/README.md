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
