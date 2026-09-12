#!/usr/bin/env python3
"""Exercise a built rgit against a fresh native copy of this source repository.

Retains the evaluation directory and phase logs for inspection. No remote publication
or changes to the source checkout are performed. Requires Git, Cargo and status JSON.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=Path)
parser.add_argument("source", type=Path, help="local Git source checkout")
parser.add_argument("destination", type=Path, help="new evaluation directory outside the source")
parser.add_argument("--source-revision", required=True, help="full source commit used to build the binary")
parser.add_argument("--branch", default="master")
args = parser.parse_args()
binary = args.binary.resolve(strict=True)
source = args.source.resolve(strict=True)
destination = args.destination.resolve()
if not re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", args.source_revision):
    parser.error("--source-revision must be a full commit ID")
if destination.is_relative_to(source):
    parser.error("evaluation destination must be outside the source checkout")
if not args.branch or args.branch.startswith("-"):
    parser.error("invalid branch")
source_tip = subprocess.check_output(
    ["git", "rev-parse", "--verify", "refs/heads/" + args.branch], cwd=source, text=True
).strip()
if source_tip != args.source_revision:
    parser.error("selected source branch differs from the supplied binary source revision")
destination.mkdir(parents=True, exist_ok=False)
logs = destination / "logs"
logs.mkdir()
native = destination / "native"
env = os.environ.copy()
env.pop("CARGO_TARGET_DIR", None)
env["RUST_BACKTRACE"] = "1"
env["RUST_LIB_BACKTRACE"] = "0"
phases = []


def run(label, command, cwd):
    print("Running " + label, flush=True)
    result = subprocess.run(command, cwd=cwd, env=env, capture_output=True, text=True)
    (logs / (label + ".stdout.log")).write_text(result.stdout)
    (logs / (label + ".stderr.log")).write_text(result.stderr)
    phases.append({"phase": label, "exit_code": result.returncode})
    if result.returncode:
        raise RuntimeError(label + " failed; inspect " + str(logs))
    return result.stdout


def rgit(label, *command, cwd=None):
    return run(label, [str(binary), *command], native if cwd is None else cwd)


def git(label, *command, cwd=None):
    return run(label, ["git", *command], source if cwd is None else cwd).strip()


def status(label):
    return json.loads(rgit(label, "status", "--json", "--as", "admin"))


rgit("clone", "git", "clone", str(source), str(native), "--branch", args.branch,
     "--domain", "public", cwd=destination)
assert not (native / ".git").exists(), "evaluation accidentally uses a Git working repository"
rgit("initial-verify", "repo", "verify", "--as", "admin")
initial = status("initial-status")
assert initial["changes"] == {"added": [], "modified": [], "deleted": [], "hidden_count": 0}
original_snapshot = initial["base_snapshot"]["id"]
source_count = int(git("source-commit-count", "rev-list", "--count", source_tip))
source_files = git("source-files", "ls-tree", "-r", "--name-only", "-z", source_tip)
source_file_count = len([name for name in source_files.split("\0") if name])

round_trip = destination / "unchanged.git"
rgit("unchanged-export", "git", "export", str(round_trip), "--as", "admin")
round_trip_tip = git("unchanged-tip", "rev-parse", "refs/heads/main", cwd=round_trip)
assert round_trip_tip == source_tip, "unchanged Git round trip changed commit identity"
git("unchanged-fsck", "fsck", "--strict", cwd=round_trip)

env["RGIT_TEST_APP_ROOT"] = str(native / "target" / "test-repositories")
Path(env["RGIT_TEST_APP_ROOT"]).mkdir(parents=True)
tests = run("workspace-tests", ["cargo", "test", "--workspace", "--locked"], native)
run("release-build", ["cargo", "build", "--release", "--locked"], native)
assert status("after-build-status")["changes"] == initial["changes"], "build/test changed tracked sources"
# Exercise the executable built inside the native checkout as well.
native_binary = native / "target" / "release" / ("rgit.exe" if os.name == "nt" else "rgit")
rebuilt_status = json.loads(run("rebuilt-binary-status", [str(native_binary), "status", "--json", "--as", "admin"], native))
assert rebuilt_status == status("original-binary-status")

readme = native / "README.md"
original_bytes = readme.read_bytes()
modified_bytes = original_bytes + b"\nEvaluation-only native edit.\n"
readme.write_bytes(modified_bytes)
rgit("set-author", "identity", "set", "Evaluation Developer", "evaluation@example.com")
assert status("edited-status")["changes"]["modified"] == ["README.md"]
rgit("snapshot", "snapshot", "--message", "Exercise native development checkout")
integrated = rgit("integrate", "line", "integrate", "main", "--as", "admin")
head = re.search(r"^line head: (snap_[0-9a-f]+)$", integrated, re.MULTILINE).group(1)
modified_git = destination / "modified.git"
rgit("modified-export", "git", "export", str(modified_git), "--as", "admin")
exported = subprocess.check_output(["git", "show", "main:README.md"], cwd=modified_git, env=env)
assert exported == modified_bytes
backup = destination / "backup"
rgit("backup", "repo", "backup", str(backup), "--as", "admin")
rgit("backup-verify", "repo", "verify", "--as", "admin", cwd=backup)
rgit("backup-restore", "workspace", "restore", "--discard-changes", "--as", "admin", cwd=backup)
assert (backup / "README.md").read_bytes() == modified_bytes
rgit("guarded-reset", "line", "reset", "main", "--to", original_snapshot,
     "--expected-head", head, "--as", "admin")
assert readme.read_bytes() == modified_bytes, "line reset changed working files"
rgit("restored-change", "change", "new", "after-evaluation-reset", "--target", "main")
rgit("restore-original", "workspace", "restore", "--discard-changes", "--as", "admin")
assert readme.read_bytes() == original_bytes
final_verify = rgit("final-verify", "repo", "verify", "--as", "admin").strip()
assert status("final-status")["changes"] == initial["changes"]
counts = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", tests)
assert counts and sum(int(passed) for passed, _, _ in counts) > 0, "no executed test results found"
report = {
    "schema_version": 1,
    "completed_at_utc": datetime.now(timezone.utc).isoformat(),
    "source_revision": source_tip,
    "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    "platform": platform.system() + " " + platform.machine(),
    "git_version": git("git-version", "--version"),
    "rust_version": run("rust-version", ["rustc", "--version"], native).strip(),
    "source_commit_count": source_count,
    "source_file_count": source_file_count,
    "unchanged_git_tip": round_trip_tip,
    "reported_test_passes_including_subprocesses": sum(int(passed) for passed, _, _ in counts),
    "reported_ignored_tests_and_helpers": sum(int(ignored) for _, _, ignored in counts),
    "final_verification": final_verify,
    "phases": phases,
}
(destination / "report.json").write_text(json.dumps(report, indent=2) + "\n")
print("Evaluation passed; report: " + str(destination / "report.json"), flush=True)
