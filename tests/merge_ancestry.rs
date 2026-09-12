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
        let repo = Self(std::env::temp_dir().join(format!("rgit-ancestry-{}", Uuid::new_v4())));
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
    fn head(&self) -> String {
        self.json("lines/main.json")["head_snapshot"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn snapshot(&self, change: &str) -> String {
        self.json(&format!("changes/{change}.json"))["current_snapshot"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

#[test]
fn integration_preserves_both_parents_and_repeated_integration_is_a_noop() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("file.txt"), "base").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    let previous_head = repo.head();
    let change = repo.change();
    fs::write(repo.0.join("file.txt"), "first edit").unwrap();
    repo.ok(&["snapshot"]);
    let incoming = repo.snapshot(&change);
    assert_eq!(
        repo.json(&format!("snapshots/{incoming}.json"))["parent_snapshot"],
        previous_head
    );
    repo.ok(&["line", "integrate"]);
    let head = repo.head();
    let merged = repo.json(&format!("snapshots/{head}.json"));
    assert_eq!(merged["parent_snapshot"], previous_head);
    assert_eq!(merged["merge_parents"], serde_json::json!([incoming]));
    let operations = fs::read_dir(repo.0.join(".rgit/operations"))
        .unwrap()
        .count();
    assert!(repo
        .ok(&["line", "integrate"])
        .contains("already integrated"));
    assert_eq!(repo.head(), head);
    assert_eq!(
        fs::read_dir(repo.0.join(".rgit/operations"))
            .unwrap()
            .count(),
        operations
    );
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn editing_an_integrated_change_uses_latest_shared_ancestor() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("file.txt"), "base").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    repo.change();
    fs::write(repo.0.join("file.txt"), "first edit").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    fs::write(repo.0.join("file.txt"), "second edit").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["merge", "preview"]);
    repo.ok(&["line", "integrate"]);
    assert!(repo.ok(&["conflict", "list"]).is_empty());
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn verifier_detects_cycles_through_merge_parents() {
    let repo = Repo::new();
    repo.change();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    let head = repo.head();
    let path = format!("snapshots/{head}.json");
    let mut snapshot = repo.json(&path);
    snapshot["merge_parents"] = serde_json::json!([head]);
    fs::write(
        repo.0.join(".rgit").join(path),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    assert!(!repo
        .run(&["repo", "verify", "--as", "admin"])
        .status
        .success());
}

#[test]
fn integration_refuses_file_directory_collisions_without_publishing_records() {
    let repo = Repo::new();
    let file_change = repo.change();
    repo.change();
    fs::create_dir(repo.0.join("a")).unwrap();
    fs::write(repo.0.join("a/b"), "nested").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate", "main"]);
    repo.ok(&["workspace", "switch", &file_change]);
    fs::remove_dir(repo.0.join("a")).unwrap();
    fs::write(repo.0.join("a"), "file").unwrap();
    repo.ok(&["snapshot"]);
    let head = repo.head();
    let records = |directory: &str| {
        fs::read_dir(repo.0.join(".rgit").join(directory))
            .unwrap()
            .count()
    };
    let snapshots = records("snapshots");
    let operations = records("operations");
    for args in [
        &["merge", "preview"][..],
        &["line", "integrate", "main"][..],
    ] {
        let output = repo.run(args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("file/directory collision"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("result: clean"));
        assert_eq!(repo.head(), head);
        assert_eq!(records("snapshots"), snapshots);
        assert_eq!(records("operations"), operations);
    }
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn integration_refuses_a_corrupt_selected_blob_without_advancing_the_line() {
    let repo = Repo::new();
    let change = repo.change();
    fs::write(repo.0.join("file"), "saved").unwrap();
    repo.ok(&["snapshot"]);
    let snapshot = repo.json(&format!("snapshots/{}.json", repo.snapshot(&change)));
    let hash = snapshot["files"][0]["hash"].as_str().unwrap();
    fs::write(repo.0.join(".rgit/blobs").join(hash), "broken").unwrap();
    let before = repo.json("lines/main.json");
    for args in [&["merge", "preview"][..], &["line", "integrate"][..]] {
        assert!(!repo.run(args).status.success());
        assert_eq!(repo.json("lines/main.json"), before);
    }
}

#[test]
fn saved_parent_resolution_cannot_publish_a_file_above_merged_descendants() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("a"), "base").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    let incoming = repo.change();
    fs::write(repo.0.join("a"), "incoming").unwrap();
    repo.ok(&["snapshot"]);
    repo.change();
    fs::remove_file(repo.0.join("a")).unwrap();
    fs::create_dir(repo.0.join("a")).unwrap();
    fs::write(repo.0.join("a/b"), "child").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    repo.ok(&["workspace", "switch", &incoming]);
    assert!(!repo.run(&["line", "integrate"]).status.success());
    let conflicts = repo.ok(&["conflict", "list"]);
    let id = conflicts.split_whitespace().next().unwrap();
    repo.ok(&["conflict", "resolve", id, "--take", "incoming"]);
    let head = repo.head();
    for args in [&["merge", "preview"][..], &["line", "integrate"][..]] {
        let output = repo.run(args);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("file/directory collision"));
        assert_eq!(repo.head(), head);
    }
    repo.ok(&["repo", "verify", "--as", "admin"]);
}
