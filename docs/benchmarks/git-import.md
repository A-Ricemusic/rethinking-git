# Git import benchmark

Measured September 12, 2026 UTC on macOS 26.5.2, arm64, with release builds and
Apple Git 2.50.1. The deterministic fixture has 100 unchanged 4 KiB binary files
and one changing revision file across 20 commits. Three imports use separate new
native repositories. Each result is verified and its snapshot count checked.

| Implementation | Median wall time | Evidence |
| --- | ---: | --- |
| Original per-file subprocess/read | 20.412 s | [Raw baseline](git-import-baseline.json) |
| Reuse verified blob admissions | 3.209 s | [Raw cache result](git-import-cache.json) |
| Cache plus persistent raw batch reader | 1.997 s | [Raw batch result](git-import-batch.json) |

The final measurement is about 10.2 times faster on this fixture. Raw reports include
individual samples, fixture Git head, measured source revision, binary checksum,
platform and Git version. These historical source revisions were recorded alongside
the measurements; the harness now requires that provenance explicitly. The final formatting/lint fix does not change the measured
algorithm. Import still verifies object identities, native blobs, trees, graph
closure and publication; cache entries hold only per-import verified blob metadata.
A regular blob reused as a symlink is still checked for a valid target.

Reproduce from the implementation being measured, using the committed harness:

```sh
cargo build --release --locked
python3 scripts/benchmark-git-import.py target/release/rgit --source-revision FULL_SOURCE_COMMIT --output /tmp/import-timing.json
```

Replace `FULL_SOURCE_COMMIT` with the full commit used to build that binary; do not
substitute the current checkout revision when timing an older binary. Use the same
harness with an earlier built binary to compare revisions. File count,
commit count, bytes and run count are configurable. Timings cover the import command,
including its integrity checks and metadata publication; fixture generation and the
extra post-import verification are excluded.

These are warm-cache wall-time measurements with highly compressible generated data
and no network. They do not establish cold-cache, large-file, memory, network,
Windows/Linux performance, or professional capacity budgets. Full-store refresh and
repeated tree verification still need representative scaling measurements.

The reader follows Git's documented [raw batch output](https://git-scm.com/docs/git-cat-file#_batch_output):
identity/type/length header, exact binary bytes, then a terminator. Requests use full
object IDs; filters, mailmap and symlink following are disabled. Tests include binary
NUL/newline payloads, adjacent frames, missing/truncated/malformed responses and
impossible allocation sizes.
