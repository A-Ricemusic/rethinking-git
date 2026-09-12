#!/usr/bin/env python3
"""Compare first/repeated snapshot RSS on macOS with explicit binary provenance."""
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
    parser.add_argument("--file-mib", type=int, default=512)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("this runner parses macOS /usr/bin/time -l; RSS units are bytes")
    if args.file_mib < 1 or args.repeats < 1:
        parser.error("file size and repeat count must be positive")
    if args.output.exists():
        parser.error("output already exists; choose a new report path")
    binaries = {}
    for side in ("baseline", "candidate"):
        revision = getattr(args, f"{side}_revision")
        if not re.fullmatch(r"[0-9a-f]{40}", revision):
            parser.error("provide a full source commit for each binary")
        path = getattr(args, f"{side}_binary").resolve(strict=True)
        binaries[side] = {"path": str(path), "source_revision": revision, "sha256": sha256(path)}
    results = []
    for repeat in range(args.repeats):
        order = ("baseline", "candidate") if repeat % 2 == 0 else ("candidate", "baseline")
        for side in order:
            with tempfile.TemporaryDirectory(prefix="rgit-snapshot-memory-") as temporary:
                root = pathlib.Path(temporary)
                binary = binaries[side]["path"]
                def run(command):
                    return subprocess.run([binary, *command], cwd=root, check=True, capture_output=True)
                run(["init"])
                run(["change", "new", "benchmark"])
                block = bytes(range(256)) * 4096
                with (root / "large.bin").open("wb") as destination:
                    for _ in range(args.file_mib):
                        destination.write(block)
                expected = sha256(root / "large.bin")
                for phase in ("first", "reuse"):
                    started = time.perf_counter()
                    result = subprocess.run(["/usr/bin/time", "-l", binary, "snapshot", "--message", phase], cwd=root, check=True, capture_output=True)
                    elapsed = time.perf_counter() - started
                    match = re.search(rb"^\s*(\d+)\s+maximum resident set size\s*$", result.stderr, re.MULTILINE)
                    if not match:
                        raise RuntimeError("missing macOS maximum RSS measurement")
                    snapshots = list((root / ".rgit/snapshots").glob("*.json"))
                    assert len(snapshots) == (1 if phase == "first" else 2)
                    for snapshot in snapshots:
                        files = json.loads(snapshot.read_text())["files"]
                        assert len(files) == 1
                        assert files[0]["hash"] == expected
                        assert files[0]["bytes"] == args.file_mib * 1048576
                    assert sha256(root / ".rgit/blobs" / expected) == expected
                    assert len(list((root / ".rgit/blobs").iterdir())) == 1
                    results.append({"repeat": repeat + 1, "phase": phase, "side": side, "peak_rss_bytes": int(match[1]), "elapsed_seconds": elapsed, "exit_code": result.returncode})
                run(["repo", "verify", "--as", "admin"])
    summary = {}
    for phase in ("first", "reuse"):
        summary[phase] = {}
        for side in binaries:
            rows = [row for row in results if row["phase"] == phase and row["side"] == side]
            summary[phase][side] = {"median_peak_rss_bytes": statistics.median(row["peak_rss_bytes"] for row in rows), "median_elapsed_seconds": statistics.median(row["elapsed_seconds"] for row in rows)}
    report = {"schema_version": 1, "platform": platform.platform(), "machine": platform.machine(), "runner_sha256": sha256(pathlib.Path(__file__)), "file_bytes": args.file_mib * 1048576, "cache": "source freshly written and hashed; reuse follows first capture and verification; binary order alternated", "binaries": {side: {key: value for key, value in binary.items() if key != "path"} for side, binary in binaries.items()}, "results": results, "summary": summary}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
