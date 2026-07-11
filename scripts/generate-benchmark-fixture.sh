#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
if [[ $(uname -s) == Darwin && -d /Library/Developer/CommandLineTools ]]; then
  export DEVELOPER_DIR=${DEVELOPER_DIR:-/Library/Developer/CommandLineTools}
fi
BIN="$ROOT/target/benchmark-tools/fixture-generator"
mkdir -p "$(dirname "$BIN")"
rustc --edition=2021 -O "$ROOT/benches/fixture_generator.rs" -o "$BIN"
exec "$BIN" "$@"
