# Benchmark methodology and budgets

The Milestone 0 benchmark suite measures the release build through its public CLI. It deliberately includes process startup, filesystem traversal, object serialization, and durable writes because those costs are visible to users.

## Reproduce a run

From the repository root:

```sh
RGIT_BENCH_RUNS=5 scripts/benchmark.sh
```

The default run uses the generated `small` and `medium` profiles and writes `benchmark-results.csv` plus `benchmark-hardware.md`. Both files are intended to be archived with a benchmark report. Select other profiles with `RGIT_BENCH_PROFILES="linux-scale monorepo"`. Large profiles require substantial free disk and are not part of a routine developer run.

Definitions:

- `init` measures metadata initialization in a populated working directory.
- `status_process_cold` is the first status process after setup. It does not claim to evict the OS page cache.
- `status_cache_warm` follows an untimed status and benefits from filesystem cache state.
- `scan_workspace` uses `diff workspace`, which traverses and hashes the work tree in the prototype.
- `snapshot` measures a one-file mutation snapshot against the generated corpus.
- `snapshot_diff` compares two manifests with one modified file.
- `merge_preview` plans a deterministic both-modified conflict without changing the line.

Each result row includes elapsed milliseconds, peak resident memory in KiB, and `.rgit` disk usage in KiB. A monotonic-enough high-resolution wall clock surrounds each process; macOS uses BSD `time -l` and Linux uses GNU `time` for peak RSS. Compare results only within a named hardware profile and operating system. Run on an otherwise idle machine, with release builds, at least five repetitions, and report median and p95. The generated payload is deterministic; IDs and operation timestamps are intentionally not.

## Fixture profiles

| Profile | Files | Bytes/file | Directories | Purpose |
|---|---:|---:|---:|---|
| small | 128 | 2 KiB | 16 | Fast developer regression loop |
| medium | 4,096 | 4 KiB | 256 | Routine CI baseline |
| linux-scale | 80,000 | 8 KiB | 4,000 | Kernel-shaped file-count stress |
| monorepo | 250,000 | 4 KiB | 10,000 | Large monorepo file-count stress |

Use `scripts/generate-benchmark-fixture.sh --custom PATH FILES BYTES_PER_FILE DIRECTORIES` for scale experiments. The names describe workload scale, not copies of Linux or any proprietary monorepo.

## Initial Milestone 0 budgets

These are regression guardrails for the current unoptimized prototype on the reference profile (Apple M1 Pro, 16 GiB RAM, local APFS). They are deliberately generous until repeated baseline data is available.

| Operation | Small p95 | Medium p95 | Peak RSS | Metadata disk budget |
|---|---:|---:|---:|---:|
| init | 300 ms | 300 ms | 64 MiB | 1 MiB |
| process-cold status / scan | 250 ms | 5,000 ms | 256 MiB | no growth |
| cache-warm status | 150 ms | 3,000 ms | 256 MiB | no growth |
| snapshot | 500 ms | 8,000 ms | 512 MiB | at most 1.25× logical payload + 16 MiB |
| snapshot diff | 150 ms | 1,000 ms | 256 MiB | no growth |
| merge preview | 250 ms | 2,000 ms | 256 MiB | no growth |

Linux-scale and monorepo runs initially establish observed baselines rather than pass/fail gates. A production object store must later replace these prototype budgets with stricter platform-specific targets. Benchmark failures should report the raw CSV and hardware profile; they must not silently weaken a budget.
