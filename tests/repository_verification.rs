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
        let repo = Self(std::env::temp_dir().join(format!("rgit-verify-{}", Uuid::new_v4())));
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
    fn saved(&self) -> String {
        let id = self.change();
        fs::write(self.0.join("file.txt"), "saved bytes").unwrap();
        self.ok(&["snapshot"]);
        self.ok(&["line", "integrate"]);
        id
    }
    fn snapshot_path(&self, change: &str) -> PathBuf {
        let change: Value = serde_json::from_slice(
            &fs::read(self.0.join(format!(".rgit/changes/{change}.json"))).unwrap(),
        )
        .unwrap();
        self.0.join(format!(
            ".rgit/snapshots/{}.json",
            change["current_snapshot"].as_str().unwrap()
        ))
    }
}

#[test]
fn verifies_saved_work_and_requires_admin_view() {
    let repo = Repo::new();
    repo.saved();
    assert!(repo
        .ok(&["repo", "verify", "--as", "admin"])
        .contains("repository verified"));
    assert!(!repo.run(&["repo", "verify"]).status.success());
}

#[test]
fn detects_blob_corruption_without_rewriting_it() {
    let repo = Repo::new();
    let change = repo.saved();
    let snapshot: Value =
        serde_json::from_slice(&fs::read(repo.snapshot_path(&change)).unwrap()).unwrap();
    let blob = repo
        .0
        .join(".rgit/blobs")
        .join(snapshot["files"][0]["hash"].as_str().unwrap());
    fs::write(&blob, "damaged").unwrap();
    assert!(!repo
        .run(&["repo", "verify", "--as", "admin"])
        .status
        .success());
    assert_eq!(fs::read(blob).unwrap(), b"damaged");
}

#[test]
fn detects_missing_references_and_cycles_without_repairing_records() {
    for cycle in [false, true] {
        let repo = Repo::new();
        let change = repo.saved();
        let path = repo.snapshot_path(&change);
        let mut snapshot: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        snapshot["parent_snapshot"] = if cycle {
            snapshot["id"].clone()
        } else {
            Value::String("snap_00000000000000000000000000000000".to_string())
        };
        let altered = serde_json::to_vec(&snapshot).unwrap();
        fs::write(&path, &altered).unwrap();
        assert!(!repo
            .run(&["repo", "verify", "--as", "admin"])
            .status
            .success());
        assert_eq!(fs::read(&path).unwrap(), altered);
    }
}

#[test]
fn detects_record_filename_identity_mismatch() {
    let repo = Repo::new();
    let change = repo.saved();
    let original = repo.snapshot_path(&change);
    fs::copy(
        original,
        repo.0
            .join(".rgit/snapshots/snap_00000000000000000000000000000000.json"),
    )
    .unwrap();
    assert!(!repo
        .run(&["repo", "verify", "--as", "admin"])
        .status
        .success());
}
