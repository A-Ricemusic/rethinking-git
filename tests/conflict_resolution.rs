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
        let repo = Self(std::env::temp_dir().join(format!("rgit-resolution-{}", Uuid::new_v4())));
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
    fn change(&self) -> String {
        self.ok(&["change", "new", "test"]);
        let workspace: Value =
            serde_json::from_slice(&fs::read(self.0.join(".rgit/workspace.json")).unwrap())
                .unwrap();
        workspace["current_change"].as_str().unwrap().to_owned()
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Repo {
    fn json(&self, path: &str) -> Value {
        serde_json::from_slice(&fs::read(self.0.join(".rgit").join(path)).unwrap()).unwrap()
    }
    fn divergent(&self) -> String {
        self.change();
        fs::write(self.0.join("file.txt"), "base").unwrap();
        self.ok(&["snapshot"]);
        self.ok(&["line", "integrate"]);
        let incoming = self.change();
        fs::write(self.0.join("file.txt"), "incoming").unwrap();
        self.ok(&["snapshot"]);
        self.change();
        fs::write(self.0.join("file.txt"), "line").unwrap();
        self.ok(&["snapshot"]);
        self.ok(&["line", "integrate"]);
        self.ok(&["workspace", "switch", &incoming, "--as", "admin"]);
        assert!(!self.run(&["line", "integrate"]).status.success());
        self.conflict()
    }
    fn conflict(&self) -> String {
        self.ok(&["conflict", "list", "--as", "admin"])
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    }
    fn head(&self) -> Value {
        let line = self.json("lines/main.json");
        self.json(&format!(
            "snapshots/{}.json",
            line["head_snapshot"].as_str().unwrap()
        ))
    }
}

#[test]
fn side_resolution_publishes_selected_content_and_operation() {
    for side in ["base", "line", "incoming", "delete"] {
        let repo = Repo::new();
        let conflict = repo.divergent();
        repo.ok(&["conflict", "resolve", &conflict, "--take", side]);
        assert!(repo.ok(&["conflict", "list"]).is_empty());
        repo.ok(&["line", "integrate"]);
        let snapshot = repo.head();
        let files = snapshot["files"].as_array().unwrap();
        if side == "delete" {
            assert!(files.is_empty());
        } else {
            let hash = files[0]["hash"].as_str().unwrap();
            assert_eq!(
                fs::read(repo.0.join(".rgit/blobs").join(hash)).unwrap(),
                side.as_bytes()
            );
        }
        assert!(repo
            .ok(&["op", "log", "--as", "admin"])
            .contains("resolve_conflict"));
    }
}

#[test]
fn changed_incoming_snapshot_invalidates_prior_resolution() {
    let repo = Repo::new();
    let conflict = repo.divergent();
    repo.ok(&["conflict", "resolve", &conflict, "--take", "incoming"]);
    fs::write(repo.0.join("file.txt"), "new incoming").unwrap();
    repo.ok(&["snapshot"]);
    let output = repo.run(&["conflict", "resolve", &conflict, "--take", "line"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("sources changed"));
    assert!(!repo.run(&["line", "integrate"]).status.success());
    assert_ne!(repo.conflict(), conflict);
}

#[test]
fn restricted_actor_cannot_resolve_or_disclose_a_conflict() {
    let repo = Repo::new();
    let conflict = repo.divergent();
    repo.ok(&["actor", "set", "outsider", "--domain", "unrelated"]);
    let before = fs::read(repo.0.join(format!(".rgit/conflicts/{conflict}.json"))).unwrap();
    let output = repo.run(&[
        "conflict", "resolve", &conflict, "--take", "incoming", "--as", "outsider",
    ]);
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("file.txt"));
    assert_eq!(
        fs::read(repo.0.join(format!(".rgit/conflicts/{conflict}.json"))).unwrap(),
        before
    );
}

#[test]
fn selecting_content_cannot_drop_a_concurrent_policy_restriction() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("file.txt"), "base").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    let incoming = repo.change();
    repo.ok(&["access", "path", "file.txt", "--domain", "admin"]);
    repo.ok(&["snapshot"]);
    repo.change();
    repo.ok(&["access", "path", "file.txt", "--domain", "public"]);
    fs::write(repo.0.join("file.txt"), "line").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    repo.ok(&["workspace", "switch", &incoming, "--as", "admin"]);
    assert!(!repo
        .run(&["line", "integrate", "--as", "admin"])
        .status
        .success());
    let conflict = repo.conflict();
    assert!(!repo
        .run(&["conflict", "resolve", &conflict, "--take", "line"])
        .status
        .success());
    repo.ok(&[
        "conflict", "resolve", &conflict, "--take", "line", "--as", "admin",
    ]);
    repo.ok(&["merge", "preview", "--as", "admin"]);
    repo.ok(&["line", "integrate", "--as", "admin"]);
    assert_eq!(
        repo.head()["files"][0]["policy"]["domains"],
        serde_json::json!(["admin"])
    );
    assert!(!repo.ok(&["line", "view"]).contains("file.txt"));
}

#[test]
fn custom_resolution_captures_bytes_once_and_survives_later_working_edits() {
    let repo = Repo::new();
    let conflict = repo.divergent();
    fs::write(repo.0.join("file.txt"), "hand-merged result\n").unwrap();
    repo.ok(&["conflict", "resolve", &conflict, "--from-working"]);
    repo.ok(&["repo", "verify", "--as", "admin"]);
    fs::write(repo.0.join("file.txt"), "later unsaved work").unwrap();
    repo.ok(&["merge", "preview"]);
    repo.ok(&["line", "integrate"]);
    let snapshot = repo.head();
    let hash = snapshot["files"][0]["hash"].as_str().unwrap();
    assert_eq!(
        fs::read(repo.0.join(".rgit/blobs").join(hash)).unwrap(),
        b"hand-merged result\n"
    );
    assert_eq!(
        fs::read(repo.0.join("file.txt")).unwrap(),
        b"later unsaved work"
    );
    repo.ok(&[
        "workspace",
        "restore",
        "--from",
        snapshot["id"].as_str().unwrap(),
        "--discard-changes",
    ]);
    assert_eq!(
        fs::read(repo.0.join("file.txt")).unwrap(),
        b"hand-merged result\n"
    );
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn custom_resolution_requires_access_and_verifies_its_stored_blob_before_integration() {
    let repo = Repo::new();
    let conflict = repo.divergent();
    repo.ok(&["actor", "set", "outsider", "--domain", "unrelated"]);
    fs::write(repo.0.join("file.txt"), "custom bytes").unwrap();
    assert!(!repo
        .run(&[
            "conflict",
            "resolve",
            &conflict,
            "--from-working",
            "--as",
            "outsider"
        ])
        .status
        .success());
    assert_eq!(
        repo.json(&format!("conflicts/{conflict}.json"))["status"],
        "unresolved"
    );
    repo.ok(&["conflict", "resolve", &conflict, "--from-working"]);
    let record = repo.json(&format!("conflicts/{conflict}.json"));
    let hash = record["replacement"]["hash"].as_str().unwrap();
    fs::write(repo.0.join(".rgit/blobs").join(hash), "corrupt").unwrap();
    let previous = repo.json("lines/main.json");
    assert!(!repo.run(&["line", "integrate"]).status.success());
    assert!(!repo
        .run(&["repo", "verify", "--as", "admin"])
        .status
        .success());
    assert_eq!(repo.json("lines/main.json"), previous);
}

#[test]
fn missing_working_file_requires_explicit_delete_and_new_sources_invalidate_custom_resolution() {
    let repo = Repo::new();
    let conflict = repo.divergent();
    fs::remove_file(repo.0.join("file.txt")).unwrap();
    assert!(!repo
        .run(&["conflict", "resolve", &conflict, "--from-working"])
        .status
        .success());
    fs::write(repo.0.join("file.txt"), "merged").unwrap();
    repo.ok(&["conflict", "resolve", &conflict, "--from-working"]);
    repo.ok(&["snapshot"]);
    assert!(!repo
        .run(&["conflict", "resolve", &conflict, "--from-working"])
        .status
        .success());
    assert!(!repo.run(&["line", "integrate"]).status.success());
}

#[test]
fn custom_content_cannot_drop_a_concurrent_policy_restriction() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("file.txt"), "base").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    let incoming = repo.change();
    repo.ok(&["access", "path", "file.txt", "--domain", "admin"]);
    repo.ok(&["snapshot"]);
    repo.change();
    repo.ok(&["access", "path", "file.txt", "--domain", "public"]);
    fs::write(repo.0.join("file.txt"), "line").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    repo.ok(&["workspace", "switch", &incoming, "--as", "admin"]);
    assert!(!repo
        .run(&["line", "integrate", "--as", "admin"])
        .status
        .success());
    let conflict = repo.conflict();
    assert!(!repo
        .run(&["conflict", "resolve", &conflict, "--from-working"])
        .status
        .success());
    repo.ok(&[
        "conflict",
        "resolve",
        &conflict,
        "--from-working",
        "--as",
        "admin",
    ]);
    repo.ok(&["merge", "preview", "--as", "admin"]);
    repo.ok(&["line", "integrate", "--as", "admin"]);
    assert_eq!(
        repo.head()["files"][0]["policy"]["domains"],
        serde_json::json!(["admin"])
    );
    assert!(!repo.ok(&["line", "view"]).contains("file.txt"));
}
