#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
if [[ $(uname -s) == Darwin && -d /Library/Developer/CommandLineTools ]]; then
  export DEVELOPER_DIR=${DEVELOPER_DIR:-/Library/Developer/CommandLineTools}
fi
RUNS=${RGIT_BENCH_RUNS:-3}
PROFILES=${RGIT_BENCH_PROFILES:-"small medium"}
OUT=${RGIT_BENCH_OUT:-"$ROOT/benchmark-results.csv"}
WORK=${RGIT_BENCH_WORK:-"${TMPDIR:-/tmp}/rgit-benchmark"}
TOOLS="$ROOT/target/benchmark-tools"
RGIT="$ROOT/target/release/rgit"
GENERATOR="$TOOLS/fixture-generator"

mkdir -p "$TOOLS" "$WORK"
cargo build --release --locked --manifest-path "$ROOT/Cargo.toml"
rustc --edition=2021 -O "$ROOT/benches/fixture_generator.rs" -o "$GENERATOR"
"$ROOT/scripts/record-benchmark-hardware.sh" "${RGIT_BENCH_HARDWARE_OUT:-$ROOT/benchmark-hardware.md}"

case $(uname -s) in
  Darwin) TIME_STYLE=bsd ;;
  Linux) TIME_STYLE=gnu ;;
  *) echo "unsupported benchmark operating system: $(uname -s)" >&2; exit 2 ;;
esac

printf 'benchmark,fixture,iteration,elapsed_ms,peak_rss_kib,metadata_kib\n' > "$OUT"

reset_fixture() {
  local profile=$1 repo=$2
  rm -rf "$repo"
  "$GENERATOR" "$profile" "$repo" >/dev/null
}

run_in() {
  local repo=$1
  shift
  (cd "$repo" && "$RGIT" "$@")
}

measure() {
  local label=$1 profile=$2 iteration=$3 repo=$4
  shift 4
  local metrics="$WORK/time.txt"
  local elapsed rss metadata started ended
  started=$(perl -MTime::HiRes=time -e 'printf "%.9f\n", time')
  if [[ $TIME_STYLE == bsd ]]; then
    (cd "$repo" && /usr/bin/time -l "$RGIT" "$@" >/dev/null) 2>"$metrics"
    rss=$(awk '/maximum resident set size/ { print int($1 / 1024); exit }' "$metrics")
  else
    (cd "$repo" && /usr/bin/time -f 'RGIT_TIME,%e,%M' "$RGIT" "$@" >/dev/null) 2>"$metrics"
    rss=$(awk -F, '$1 == "RGIT_TIME" { print $3; exit }' "$metrics")
  fi
  ended=$(perl -MTime::HiRes=time -e 'printf "%.9f\n", time')
  elapsed=$(awk -v start="$started" -v end="$ended" 'BEGIN { print (end - start) * 1000 }')
  metadata=0
  if [[ -d "$repo/.rgit" ]]; then
    metadata=$(du -sk "$repo/.rgit" | awk '{print $1}')
  fi
  if [[ -z ${elapsed:-} || -z ${rss:-} ]]; then
    echo "failed to parse /usr/bin/time output for $label" >&2
    sed -n '1,80p' "$metrics" >&2
    exit 1
  fi
  printf '%s,%s,%s,%.3f,%s,%s\n' "$label" "$profile" "$iteration" "$elapsed" "$rss" "$metadata" >> "$OUT"
}

snapshot_id() {
  awk '/^created snapshot / { print $3 }'
}

change_id() {
  awk '/^created change / { print $3 }'
}

for profile in $PROFILES; do
  repo="$WORK/$profile"
  for iteration in $(seq 1 "$RUNS"); do
    reset_fixture "$profile" "$repo"
    measure init "$profile" "$iteration" "$repo" init

    run_in "$repo" change new baseline >/dev/null
    run_in "$repo" snapshot --message baseline >/dev/null
    measure status_process_cold "$profile" "$iteration" "$repo" status
    run_in "$repo" status >/dev/null
    measure status_cache_warm "$profile" "$iteration" "$repo" status
    measure scan_workspace "$profile" "$iteration" "$repo" diff workspace

    printf '\nbenchmark mutation\n' >> "$repo/src/d00000/file_00000000.txt"
    measure snapshot "$profile" "$iteration" "$repo" snapshot --message measured

    old=$(run_in "$repo" snapshot --message diff-old | snapshot_id)
    printf '\nsecond benchmark mutation\n' >> "$repo/src/d00000/file_00000000.txt"
    new=$(run_in "$repo" snapshot --message diff-new | snapshot_id)
    measure snapshot_diff "$profile" "$iteration" "$repo" diff snapshot "$old" "$new"

    reset_fixture "$profile" "$repo"
    run_in "$repo" init >/dev/null
    run_in "$repo" change new merge-base >/dev/null
    run_in "$repo" snapshot --message merge-base >/dev/null
    run_in "$repo" line integrate main >/dev/null
    target="$repo/src/d00000/file_00000000.txt"
    cp "$target" "$WORK/original.txt"
    incoming=$(run_in "$repo" change new incoming | change_id)
    printf '\nincoming side\n' >> "$target"
    run_in "$repo" snapshot --message incoming >/dev/null
    cp "$WORK/original.txt" "$target"
    run_in "$repo" change new line-advance >/dev/null
    printf '\nline side\n' >> "$target"
    run_in "$repo" snapshot --message line-advance >/dev/null
    run_in "$repo" line integrate main >/dev/null
    measure merge_preview "$profile" "$iteration" "$repo" merge preview "$incoming" --into main
  done
done

echo "benchmark results: $OUT"
