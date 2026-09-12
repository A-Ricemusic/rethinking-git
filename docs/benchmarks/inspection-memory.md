# Inspection memory measurement

September 12, 2026, macOS arm64, system Python 3.9.6, Rust 1.97 release builds.
This is a narrow memory comparison on one deterministic 512 MiB binary file, not a
representative repository throughput or release qualification benchmark.

Baseline: `66432ffcf9b6ffe7fc543ec57b61b68ed5869369`.
Candidate: `da7aaac761644345207670cd24e33b4620d77f27`.
The candidate contains the streaming implementation before later unrelated merge
features were combined. Raw reports record binary and runner SHA-256 digests and
all samples. Source revisions are supplied build provenance, not an attestation.

Each fixture is created and captured with the baseline binary. The second fixture
also integrates and exports to Git, binding saved snapshots to Git commit identities.
Both binaries must produce identical inspection output. Commands are warmed before
measurement, run three times, and alternate binary order. Maximum resident set size
comes from macOS `/usr/bin/time -l` in bytes. No local builds/tests ran during the
retained measurement samples; this was a shared development machine, not an isolated
benchmark host. Timings are retained for reproducibility, not a general speed claim.

| Fixture | Command | Baseline median peak RSS | Candidate median peak RSS | Reduction |
| --- | --- | --- | --- | --- |
| Native history | `status` | 516.45 MiB | 4.47 MiB | 115.6× |
| Native history | `verify` | 516.44 MiB | 4.44 MiB | 116.4× |
| Git-bound history | `status` | 516.39 MiB | 4.53 MiB | 114.0× |
| Git-bound history | `verify` | 516.53 MiB | 4.52 MiB | 114.4× |

Raw reports: [native history](inspection-native-2026-09-12.json),
[Git-bound history](inspection-git-2026-09-12.json).

Regular-file status/diff inspection and blob verification now hash through a 64 KiB
buffer. Git-provenance verification computes blob/tree identities without retaining
blob payloads. Native SHA-256, Git SHA-1/SHA-256, lengths and unsafe-blob checks remain
validated. Short/interrupted reads and read failures have regression coverage;
Git import/export and corruption suites validate interoperability.

Memory still scales with repository metadata. At these recorded revisions, snapshot
capture retained file contents; subsequent regular-file streaming capture has its
own benchmark runner, `scripts/benchmark-snapshot-memory.py`. Symlink-content
validation, working-tree checkout journals, and Git export retain in-memory content
paths. This result does not establish bounded memory for those operations or qualify
large repositories generally.

## Reproduction

Build each recorded revision with `cargo build --release --locked` in a separate
worktree. Keep those binaries unchanged and use the committed runner:

```sh
python3 scripts/benchmark-inspection-memory.py \
  --baseline-binary /path/to/baseline/target/release/rgit \
  --baseline-revision 66432ffcf9b6ffe7fc543ec57b61b68ed5869369 \
  --candidate-binary /path/to/candidate/target/release/rgit \
  --candidate-revision da7aaac761644345207670cd24e33b4620d77f27 \
  --file-mib 512 --repeats 3 --output /new/path/native.json
```

Repeat with `--git-provenance` and a new output path for the Git-bound fixture.
The runner requires macOS and Python 3.9+, refuses an existing report path, and
removes its temporary synthetic repositories. Git is needed for the Git-bound case.
