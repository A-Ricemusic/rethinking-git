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
        fs::write(
            self.0.join(".rgit/workspace.json"),
            serde_json::to_vec(&serde_json::json!({"current_change": incoming})).unwrap(),
        )
        .unwrap();
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
    fs::write(
        repo.0.join(".rgit/workspace.json"),
        serde_json::to_vec(&serde_json::json!({"current_change": incoming})).unwrap(),
    )
    .unwrap();
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
