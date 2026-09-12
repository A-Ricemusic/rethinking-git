#!/usr/bin/env python3
"""Build every library target without repository-level sources or specifications."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
# Retain the exact workspace resolution so --locked remains meaningful.
# The excluded CLI has only a placeholder target, never its real sources.
manifest = (root / "Cargo.toml").read_text()
with tempfile.TemporaryDirectory(prefix="rgit-library-isolation-") as temporary:
    bundle = Path(temporary)
    (bundle / "Cargo.toml").write_text(manifest)
    (bundle / "src").mkdir()
    (bundle / "src" / "main.rs").write_text("fn main() {}\n")
    shutil.copy2(root / "Cargo.lock", bundle / "Cargo.lock")
    shutil.copytree(root / "crates", bundle / "crates", ignore=shutil.ignore_patterns("target"))
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(root / "target" / "library-isolation")
    subprocess.run(
        ["cargo", "check", "--workspace", "--exclude", "rethinking-git", "--all-targets", "--locked"],
        cwd=bundle, env=env, check=True,
    )
print("All library targets build without repository-level files.")
