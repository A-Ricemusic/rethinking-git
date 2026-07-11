# Benchmark report 0001 — Milestone 0 harness

Status: baseline complete.

## Reference profile

- Apple M1 Pro, 16 GiB RAM
- macOS 26.5.1, arm64, local APFS
- Rust 1.97.0

## Results

Five iterations per fixture were run from the release build. p95 uses nearest rank, which is the maximum observation for this deliberately small initial sample.

| Operation | Small median / p95 | Medium median / p95 | Maximum RSS | Maximum `.rgit` disk |
|---|---:|---:|---:|---:|
| init | 18.592 / 232.553 ms | 17.731 / 18.456 ms | 2.3 MiB | 28 KiB |
| process-cold status | 20.768 / 21.612 ms | 148.718 / 195.144 ms | 8.2 MiB | 17,496 KiB |
| cache-warm status | 20.546 / 21.662 ms | 145.303 / 198.878 ms | 8.6 MiB | 17,496 KiB |
| workspace scan/diff | 20.719 / 21.150 ms | 148.306 / 179.989 ms | 8.6 MiB | 17,496 KiB |
| snapshot | 21.157 / 22.393 ms | 158.990 / 163.292 ms | 6.9 MiB | 18,580 KiB |
| snapshot diff | 17.202 / 17.484 ms | 24.109 / 49.803 ms | 7.8 MiB | 20,740 KiB |
| merge preview | 17.148 / 18.111 ms | 27.695 / 28.176 ms | 13.8 MiB | 21,832 KiB |

All initial latency and memory budgets pass. Disk is reported at the point of each measurement and includes setup history, prior snapshots, blobs, and operations; it is not incremental cost for the named operation. The first small `init` observation was a 232.553 ms outlier while the other four observations were 17–19 ms, illustrating why future reports should use more samples.

## Validation performed

The fixture generator's deterministic unit test passed and generated 128 files with exactly 262,144 logical bytes for the small profile. Shell syntax checks passed. The full repository suite passed: 29 binary unit tests and five CLI integration/golden tests. The benchmark behavior completed successfully.

Raw observations and environment details are stored in `0001-results.csv` and `0001-hardware.md`. Reproduce with:

```sh
RGIT_BENCH_RUNS=5 RGIT_BENCH_OUT=docs/benchmark-reports/0001-results.csv \
  RGIT_BENCH_HARDWARE_OUT=docs/benchmark-reports/0001-hardware.md \
  scripts/benchmark.sh
```

Review and commit the raw result and hardware files alongside an updated summary of median and p95 values.
