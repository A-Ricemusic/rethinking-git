# Snapshot memory measurement

September 12, 2026, macOS arm64, Python 3.9.6, Rust 1.97 release builds.
One deterministic 512 MiB binary file, three repetitions per binary, fresh native
repository per repetition. The second snapshot reuses the first blob. The runner
checks exact saved digest/length, blob contents, absence of publication temporaries,
and repository verification. Source is freshly written and hashed before capture;
binary order alternates. No local builds/tests ran during these retained samples.
This was a shared development machine, not an isolated benchmark host.

Baseline: `36acf91dfc562580437c12ac8e06e3988b240586`.
Candidate: `98d39cabddab1f7b7a1e57bc1b801a551b324964`.
The [raw report](snapshot-memory-2026-09-12.json) records all samples and source,
binary and runner digests. Source revisions are supplied build provenance, not an
attestation. Peak RSS comes from macOS `/usr/bin/time -l` in bytes.

| Snapshot | Baseline median peak RSS | Candidate median peak RSS | Reduction |
| --- | --- | --- | --- |
| first | 516.62 MiB | 4.55 MiB | 113.6× |
| reuse | 1027.62 MiB | 4.72 MiB | 217.8× |

Regular-file capture now uses a 64 KiB copy/hash buffer and fixed-size exact
comparison buffers. It needs filesystem hard-link support and temporary disk space
up to the file size, including for reuse. This is a narrow memory result, not a
throughput improvement or release qualification claim. Raw timings are retained;
reuse adds temporary writes. Memory still scales with metadata. Symlink validation,
checkout journals and Git export retain separate in-memory content paths.

## Reproduction

Build each recorded revision with `cargo build --release --locked` in separate
worktrees, then run on macOS with Python 3.9+:

```sh
python3 scripts/benchmark-snapshot-memory.py \
  --baseline-binary /path/to/baseline/target/release/rgit \
  --baseline-revision 36acf91dfc562580437c12ac8e06e3988b240586 \
  --candidate-binary /path/to/candidate/target/release/rgit \
  --candidate-revision 98d39cabddab1f7b7a1e57bc1b801a551b324964 \
  --file-mib 512 --repeats 3 --output /new/path/snapshot.json
```

The runner refuses existing output paths and removes its synthetic repositories.
