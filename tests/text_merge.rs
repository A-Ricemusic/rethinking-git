use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use uuid::Uuid;

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let repo = Self(std::env::temp_dir().join(format!("rgit-text-merge-{}", Uuid::new_v4())));
        fs::create_dir(&repo.0).unwrap();
        repo.ok(&["init"]);
        repo
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rgit"))
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn json(&self, key: &str) -> Value {
        serde_json::from_slice(&fs::read(self.0.join(".rgit").join(key)).unwrap()).unwrap()
    }
    fn change(&self) -> String {
        self.ok(&["change", "new", "test"]);
        self.json("workspace.json")["current_change"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn divergent(&self, base: &[u8], line: &[u8], incoming: &[u8]) {
        self.change();
        fs::write(self.0.join("file"), base).unwrap();
        self.ok(&["snapshot"]);
        self.ok(&["line", "integrate"]);
        let change = self.change();
        fs::write(self.0.join("file"), incoming).unwrap();
        self.ok(&["snapshot"]);
        self.change();
        fs::write(self.0.join("file"), line).unwrap();
        self.ok(&["snapshot"]);
        self.ok(&["line", "integrate"]);
        self.ok(&["workspace", "switch", &change]);
    }
    fn head(&self) -> Value {
        let id = self.json("lines/main.json")["head_snapshot"]
            .as_str()
            .unwrap()
            .to_owned();
        self.json(&format!("snapshots/{id}.json"))
    }
    fn blob_count(&self) -> usize {
        fs::read_dir(self.0.join(".rgit/blobs")).unwrap().count()
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn independent_saved_text_edits_merge_without_git_and_preview_does_not_publish_blobs() {
    let repo = Repo::new();
    let base = b"one\ntwo\nthree\nfour\nfive\n";
    let line = b"ONE\ntwo\nthree\nfour\nfive\n";
    let incoming = b"one\ntwo\nthree\nfour\nFIVE\n";
    let expected = b"ONE\ntwo\nthree\nfour\nFIVE\n";
    repo.divergent(base, line, incoming);
    let head = repo.json("lines/main.json");
    let blobs = repo.blob_count();
    assert!(repo.ok(&["merge", "preview"]).contains("result: clean"));
    assert_eq!(repo.blob_count(), blobs);
    assert_eq!(repo.json("lines/main.json"), head);
    let output = Command::new(env!("CARGO_BIN_EXE_rgit"))
        .args(["line", "integrate"])
        .env("PATH", "")
        .current_dir(&repo.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snapshot = repo.head();
    let hash = snapshot["files"][0]["hash"].as_str().unwrap();
    assert_eq!(
        fs::read(repo.0.join(".rgit/blobs").join(hash)).unwrap(),
        expected
    );
    assert_eq!(fs::read(repo.0.join("file")).unwrap(), incoming);
    assert_eq!(snapshot["merge_parents"].as_array().unwrap().len(), 1);
    repo.ok(&["repo", "verify", "--as", "admin"]);
    for (name, content) in [
        ("base", base.as_slice()),
        ("line", line.as_slice()),
        ("incoming", incoming.as_slice()),
    ] {
        fs::write(repo.0.join(name), content).unwrap();
    }
    let oracle = Command::new("git")
        .args(["merge-file", "-p", "line", "base", "incoming"])
        .current_dir(&repo.0)
        .output()
        .unwrap();
    assert!(oracle.status.success());
    assert_eq!(oracle.stdout, expected);
}

#[test]
fn binary_invalid_utf8_large_and_overlapping_edits_remain_explicit_conflicts() {
    let large_base = format!("{}\nbase\n", "x".repeat(1_048_576));
    let large_line = large_base.replace("base", "line");
    let large_incoming = large_base.replace("base", "incoming");
    for (base, line, incoming) in [
        (&b"a\0b"[..], &b"A\0b"[..], &b"a\0B"[..]),
        (&b"a\xffb"[..], &b"A\xffb"[..], &b"a\xffB"[..]),
        (&b"base\n"[..], &b"line\n"[..], &b"incoming\n"[..]),
        (
            large_base.as_bytes(),
            large_line.as_bytes(),
            large_incoming.as_bytes(),
        ),
    ] {
        let repo = Repo::new();
        repo.divergent(base, line, incoming);
        let before = repo.json("lines/main.json");
        let blobs = repo.blob_count();
        assert!(repo.ok(&["merge", "preview"]).contains("result: conflicts"));
        assert!(!repo.run(&["line", "integrate"]).status.success());
        assert_eq!(repo.json("lines/main.json"), before);
        assert_eq!(repo.blob_count(), blobs);
        repo.ok(&["repo", "verify", "--as", "admin"]);
    }
}

#[test]
fn corrupt_text_sources_fail_before_preview_or_integration_can_claim_success() {
    let repo = Repo::new();
    repo.divergent(b"a\nb\nc\n", b"A\nb\nc\n", b"a\nb\nC\n");
    let snapshot = repo.head();
    let hash = snapshot["files"][0]["hash"].as_str().unwrap();
    fs::write(repo.0.join(".rgit/blobs").join(hash), "corrupt").unwrap();
    let head = repo.json("lines/main.json");
    for args in [&["merge", "preview"][..], &["line", "integrate"][..]] {
        let output = repo.run(args);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("verification"));
        assert_eq!(repo.json("lines/main.json"), head);
    }
}

#[test]
fn mixed_manual_and_automatic_results_publish_together_after_resolution() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("auto"), "a\nb\nc\n").unwrap();
    fs::write(repo.0.join("manual"), "base\n").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    let incoming = repo.change();
    fs::write(repo.0.join("auto"), "a\nb\nC\n").unwrap();
    fs::write(repo.0.join("manual"), "incoming\n").unwrap();
    repo.ok(&["snapshot"]);
    repo.change();
    fs::write(repo.0.join("auto"), "A\nb\nc\n").unwrap();
    fs::write(repo.0.join("manual"), "line\n").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    repo.ok(&["workspace", "switch", &incoming]);
    let head = repo.json("lines/main.json");
    let blobs = repo.blob_count();
    assert!(!repo.run(&["line", "integrate"]).status.success());
    assert_eq!(repo.json("lines/main.json"), head);
    assert_eq!(repo.blob_count(), blobs);
    let conflicts = repo.ok(&["conflict", "list"]);
    assert_eq!(conflicts.lines().count(), 1);
    let id = conflicts.split_whitespace().next().unwrap();
    assert!(conflicts.contains("manual"));
    repo.ok(&["conflict", "resolve", id, "--take", "incoming"]);
    repo.ok(&["line", "integrate"]);
    let snapshot = repo.head();
    for file in snapshot["files"].as_array().unwrap() {
        let expected = if file["path"] == "auto" {
            "A\nb\nC\n"
        } else {
            "incoming\n"
        };
        assert_eq!(
            fs::read(
                repo.0
                    .join(".rgit/blobs")
                    .join(file["hash"].as_str().unwrap())
            )
            .unwrap(),
            expected.as_bytes()
        );
    }
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn concurrent_policy_changes_require_authorized_explicit_resolution() {
    let repo = Repo::new();
    repo.divergent(b"a\nb\nc\n", b"A\nb\nc\n", b"a\nb\nC\n");
    repo.ok(&["access", "path", "file", "--domain", "admin"]);
    repo.ok(&["snapshot"]);
    let denied = repo.run(&["merge", "preview"]);
    assert!(!denied.status.success());
    assert!(!String::from_utf8_lossy(&denied.stdout).contains("result: clean"));
    assert!(repo
        .ok(&["merge", "preview", "--as", "admin"])
        .contains("result: conflicts"));
    assert!(!repo
        .run(&["line", "integrate", "--as", "admin"])
        .status
        .success());
    let conflicts = repo.ok(&["conflict", "list", "--as", "admin"]);
    let id = conflicts.split_whitespace().next().unwrap();
    repo.ok(&["conflict", "resolve", id, "--take", "line", "--as", "admin"]);
    repo.ok(&["line", "integrate", "--as", "admin"]);
    assert_eq!(
        repo.head()["files"][0]["policy"]["domains"],
        serde_json::json!(["admin"])
    );
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

#[cfg(unix)]
#[test]
fn concurrent_mode_changes_are_not_automatically_resolved_as_text() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    repo.divergent(b"a\nb\nc\n", b"A\nb\nc\n", b"a\nb\nC\n");
    fs::set_permissions(repo.0.join("file"), fs::Permissions::from_mode(0o755)).unwrap();
    repo.ok(&["snapshot"]);
    assert!(repo.ok(&["merge", "preview"]).contains("result: conflicts"));
    assert!(!repo.run(&["line", "integrate"]).status.success());
}

#[test]
fn corrupt_preexisting_result_blob_is_refused_without_advancing_the_line() {
    use sha2::{Digest, Sha256};
    let repo = Repo::new();
    repo.divergent(b"a\nb\nc\n", b"A\nb\nc\n", b"a\nb\nC\n");
    let hash = hex::encode(Sha256::digest(b"A\nb\nC\n"));
    let path = repo.0.join(".rgit/blobs").join(hash);
    fs::write(&path, vec![b'x'; 1_048_577]).unwrap();
    let head = repo.json("lines/main.json");
    let output = repo.run(&["line", "integrate"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("existing merge blob failed verification")
    );
    assert_eq!(repo.json("lines/main.json"), head);
    assert_eq!(fs::metadata(path).unwrap().len(), 1_048_577);
}
