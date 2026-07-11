#!/usr/bin/env bash
set -euo pipefail

OUT=${1:-benchmark-hardware.md}
os=$(uname -sr)
arch=$(uname -m)
logical_cpus=$(getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.logicalcpu)

case $(uname -s) in
  Darwin)
    cpu=$(sysctl -n machdep.cpu.brand_string)
    memory_bytes=$(sysctl -n hw.memsize)
    device=$(df . | awk 'NR == 2 {print $1}')
    filesystem=$(diskutil info "$device" 2>/dev/null | awk -F: '/File System Personality/ {sub(/^ +/, "", $2); print $2; exit}' || true)
    ;;
  Linux)
    cpu=$(awk -F: '/model name/ {sub(/^ /, "", $2); print $2; exit}' /proc/cpuinfo)
    memory_bytes=$(awk '/MemTotal/ {print $2 * 1024; exit}' /proc/meminfo)
    filesystem=$(findmnt -n -o FSTYPE . 2>/dev/null || true)
    ;;
  *) echo "unsupported operating system: $(uname -s)" >&2; exit 2 ;;
esac

disk_free_kib=$(df -k . | awk 'NR == 2 {print $4}')
rustc_version=$(rustc --version)
cargo_version=$(cargo --version)

{
  echo '# Benchmark hardware profile'
  echo
  echo "- OS: $os"
  echo "- Architecture: $arch"
  echo "- CPU: $cpu"
  echo "- Logical CPUs: $logical_cpus"
  echo "- Memory bytes: $memory_bytes"
  echo "- Filesystem: ${filesystem:-unknown}"
  echo "- Free disk KiB at start: $disk_free_kib"
  echo "- Rust: $rustc_version"
  echo "- Cargo: $cargo_version"
} > "$OUT"

echo "hardware profile: $OUT"
