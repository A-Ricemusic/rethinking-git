#!/usr/bin/env python3
"""Measure warm-cache status/verification RSS on a synthetic file (macOS only)."""
import argparse
import hashlib
import json
import pathlib
import platform
import re
import statistics
import subprocess
import tempfile
import time


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1048576), b""):
            digest.update(block)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for side in ("baseline", "candidate"):
        parser.add_argument(f"--{side}-binary", type=pathlib.Path, required=True)
        parser.add_argument(f"--{side}-revision", required=True)
    parser.add_argument("--git-provenance", action="store_true", help="bind saved history to Git identities before measuring (requires Git)")
    parser.add_argument("--file-mib", type=int, default=512)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("this runner parses macOS /usr/bin/time -l; RSS units are bytes")
    if args.file_mib < 1 or args.repeats < 1:
        parser.error("file size and repeat count must be positive")
    binaries = {}
    for side in ("baseline", "candidate"):
        revision = getattr(args, f"{side}_revision")
        if not re.fullmatch(r"[0-9a-f]{40}", revision):
            parser.error("provide a full source commit for each binary")
        path = getattr(args, f"{side}_binary").resolve(strict=True)
        binaries[side] = {"path": str(path), "source_revision": revision, "sha256": sha256(path)}
    if args.output.exists():
        parser.error("output already exists; choose a new report path")
    results = []
    commands = {"status": ["status", "--json"], "verify": ["repo", "verify", "--as", "admin"]}
    with tempfile.TemporaryDirectory(prefix="rgit-inspection-memory-") as temporary:
        root = pathlib.Path(temporary)
        baseline = binaries["baseline"]["path"]
        def run(binary, command):
            return subprocess.run([binary, *command], cwd=root, check=True, capture_output=True)
        run(baseline, ["init"])
        run(baseline, ["change", "new", "benchmark"])
        block = bytes(range(256)) * 4096
        with (root / "large.bin").open("wb") as destination:
            for _ in range(args.file_mib):
                destination.write(block)
        run(baseline, ["snapshot", "--message", "large deterministic binary"])
        if args.git_provenance:
            run(baseline, ["line", "integrate"])
            # A sibling destination is outside the native repository.
            with tempfile.TemporaryDirectory(prefix="rgit-inspection-export-") as export_parent:
                run(baseline, ["git", "export", str(pathlib.Path(export_parent) / "export.git"), "--author", "Memory Benchmark <benchmark@example.invalid>", "--as", "admin"])
        expected = {}
        for name, command in commands.items():
            expected[name] = run(baseline, command).stdout
            assert run(binaries["candidate"]["path"], command).stdout == expected[name]
        for repeat in range(args.repeats):
            order = ("baseline", "candidate") if repeat % 2 == 0 else ("candidate", "baseline")
            for name, command in commands.items():
                for side in order:
                    started = time.perf_counter()
                    result = subprocess.run(["/usr/bin/time", "-l", binaries[side]["path"], *command], cwd=root, check=True, capture_output=True)
                    elapsed = time.perf_counter() - started
                    assert result.stdout == expected[name], "inspection output changed"
                    match = re.search(rb"^\s*(\d+)\s+maximum resident set size\s*$", result.stderr, re.MULTILINE)
                    if not match:
                        raise RuntimeError("missing macOS maximum RSS measurement")
                    results.append({"repeat": repeat + 1, "command": name, "side": side, "peak_rss_bytes": int(match[1]), "elapsed_seconds": elapsed, "exit_code": result.returncode})
    summary = {}
    for name in commands:
        summary[name] = {}
        for side in binaries:
            rows = [row for row in results if row["command"] == name and row["side"] == side]
            summary[name][side] = {"median_peak_rss_bytes": statistics.median(row["peak_rss_bytes"] for row in rows), "median_elapsed_seconds": statistics.median(row["elapsed_seconds"] for row in rows)}
    report = {"schema_version": 1, "platform": platform.platform(), "machine": platform.machine(), "runner_sha256": sha256(pathlib.Path(__file__)), "file_bytes": args.file_mib * 1048576, "git_provenance": args.git_provenance, "git_version": subprocess.check_output(["git", "--version"], text=True).strip() if args.git_provenance else None, "cache": "warm; commands warmed before measurement, binary order alternated", "binaries": {side: {key: value for key, value in binary.items() if key != "path"} for side, binary in binaries.items()}, "results": results, "summary": summary}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
