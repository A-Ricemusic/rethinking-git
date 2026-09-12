#!/usr/bin/env python3
"""Reproducible CLI import timing; requires an already-built rgit binary and Git."""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import platform
import re
import statistics
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=pathlib.Path)
parser.add_argument("--files", type=int, default=100)
parser.add_argument("--commits", type=int, default=20)
parser.add_argument("--bytes", type=int, default=4096)
parser.add_argument("--runs", type=int, default=3)
parser.add_argument("--source-revision", required=True, help="Full source commit used to build the supplied binary")
parser.add_argument("--output", type=pathlib.Path, required=True)
args = parser.parse_args()
if min(args.files, args.commits, args.bytes, args.runs) < 1:
    parser.error("fixture sizes and runs must be positive")
if not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", args.source_revision):
    parser.error("source revision must be a full Git object ID")
binary = args.binary.resolve(strict=True)
env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
           GIT_AUTHOR_DATE="2000-01-01T00:00:00+0000", GIT_COMMITTER_DATE="2000-01-01T00:00:00+0000")

def run(cwd, *command):
    result = subprocess.run(command, cwd=cwd, env=env, capture_output=True, check=True)
    return result.stdout.decode().strip()

samples = []
with tempfile.TemporaryDirectory(prefix="rgit-import-benchmark-") as temporary:
    root = pathlib.Path(temporary)
    source = root / "source"
    source.mkdir()
    run(source, "git", "init", "-b", "main")
    for number in range(args.files):
        chunk = hashlib.sha256(str(number).encode()).digest()
        (source / f"file-{number:06}.bin").write_bytes((chunk * ((args.bytes + 31) // 32))[:args.bytes])
    for revision in range(args.commits):
        (source / "revision.txt").write_text(str(revision), encoding="utf-8")
        run(source, "git", "add", ".")
        run(source, "git", "-c", "user.name=Benchmark", "-c", "user.email=benchmark@example.test",
            "commit", "-m", f"revision {revision}")
    source_head = run(source, "git", "rev-parse", "HEAD")
    for number in range(args.runs):
        native = root / f"native-{number}"
        native.mkdir()
        run(native, str(binary), "init")
        start = time.perf_counter()
        run(native, str(binary), "git", "import", str(source), "--as", "admin", "--domain", "public")
        samples.append(time.perf_counter() - start)
        run(native, str(binary), "repo", "verify", "--as", "admin")
        if len(list((native / ".rgit" / "snapshots").glob("*.json"))) != args.commits:
            raise RuntimeError("imported snapshot count differs from fixture")
        print(f"run {number + 1}: {samples[-1]:.3f}s", flush=True)
report = dict(schema=1, source_revision=args.source_revision, measured_at=datetime.datetime.now(datetime.timezone.utc).isoformat(),
              platform=platform.platform(), machine=platform.machine(),
              binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
              git_version=run(pathlib.Path.cwd(), "git", "--version"),
              files=args.files, commits=args.commits, bytes_per_file=args.bytes,
              fixture_git_head=source_head, seconds=samples, median_seconds=statistics.median(samples),
              notes="Wall time includes CLI verification/publication; warm OS cache, no network. Not a production capacity qualification.")
args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
