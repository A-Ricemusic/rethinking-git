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
        let repo = Self(std::env::temp_dir().join(format!("rgit-checkout-{}", Uuid::new_v4())));
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
    fn branches(&self) -> (String, String) {
        let first = self.change();
        fs::write(self.0.join("file.txt"), "first").unwrap();
        self.ok(&["snapshot"]);
        self.ok(&["line", "integrate"]);
        let second = self.change();
        fs::write(self.0.join("file.txt"), "second").unwrap();
        fs::create_dir(self.0.join("nested")).unwrap();
        fs::write(self.0.join("nested/new.txt"), "new").unwrap();
        self.ok(&["snapshot"]);
        (first, second)
    }
    fn current(&self) -> String {
        let json: Value =
            serde_json::from_slice(&fs::read(self.0.join(".rgit/workspace.json")).unwrap())
                .unwrap();
        json["current_change"].as_str().unwrap().to_string()
    }
}

#[test]
fn switch_restores_bytes_additions_and_deletions_preserving_untracked_files() {
    let repo = Repo::new();
    let (first, second) = repo.branches();
    fs::write(repo.0.join("untracked.txt"), "keep me").unwrap();
    repo.ok(&["workspace", "switch", &first]);
    assert_eq!(repo.current(), first);
    assert_eq!(fs::read(repo.0.join("file.txt")).unwrap(), b"first");
    assert!(!repo.0.join("nested/new.txt").exists());
    assert_eq!(fs::read(repo.0.join("untracked.txt")).unwrap(), b"keep me");
    repo.ok(&["workspace", "switch", &second]);
    assert_eq!(fs::read(repo.0.join("file.txt")).unwrap(), b"second");
    assert_eq!(fs::read(repo.0.join("nested/new.txt")).unwrap(), b"new");
}

#[test]
fn dirty_worktree_requires_explicit_restore_and_switch_is_atomic_on_refusal() {
    let repo = Repo::new();
    let (first, second) = repo.branches();
    fs::write(repo.0.join("file.txt"), "unsaved edit").unwrap();
    assert!(!repo.run(&["workspace", "switch", &first]).status.success());
    assert_eq!(repo.current(), second);
    assert_eq!(fs::read(repo.0.join("nested/new.txt")).unwrap(), b"new");
    assert!(!repo.run(&["workspace", "restore"]).status.success());
    assert_eq!(fs::read(repo.0.join("file.txt")).unwrap(), b"unsaved edit");
    repo.ok(&["workspace", "restore", "--discard-changes"]);
    assert_eq!(fs::read(repo.0.join("file.txt")).unwrap(), b"second");
}

#[test]
fn untracked_collision_is_preserved_without_changing_the_workspace_pointer() {
    let repo = Repo::new();
    let (first, second) = repo.branches();
    repo.ok(&["workspace", "switch", &first]);
    fs::write(repo.0.join("nested/new.txt"), "untracked edit").unwrap();
    assert!(!repo.run(&["workspace", "switch", &second]).status.success());
    assert_eq!(repo.current(), first);
    assert_eq!(fs::read(repo.0.join("file.txt")).unwrap(), b"first");
    assert_eq!(
        fs::read(repo.0.join("nested/new.txt")).unwrap(),
        b"untracked edit"
    );
}

#[test]
fn corrupt_blob_is_rejected_before_working_files_change() {
    let repo = Repo::new();
    let (first, second) = repo.branches();
    let change: Value = serde_json::from_slice(
        &fs::read(repo.0.join(format!(".rgit/changes/{first}.json"))).unwrap(),
    )
    .unwrap();
    let id = change["current_snapshot"].as_str().unwrap();
    let snapshot: Value = serde_json::from_slice(
        &fs::read(repo.0.join(format!(".rgit/snapshots/{id}.json"))).unwrap(),
    )
    .unwrap();
    fs::write(
        repo.0
            .join(".rgit/blobs")
            .join(snapshot["files"][0]["hash"].as_str().unwrap()),
        "corrupt",
    )
    .unwrap();
    assert!(!repo.run(&["workspace", "switch", &first]).status.success());
    assert_eq!(repo.current(), second);
    assert_eq!(fs::read(repo.0.join("file.txt")).unwrap(), b"second");
}

#[cfg(unix)]
#[test]
fn restore_refuses_symlink_traversal() {
    let repo = Repo::new();
    repo.branches();
    fs::remove_file(repo.0.join("nested/new.txt")).unwrap();
    fs::remove_dir(repo.0.join("nested")).unwrap();
    let external = repo.0.join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("new.txt"), "outside").unwrap();
    std::os::unix::fs::symlink(&external, repo.0.join("nested")).unwrap();
    assert!(!repo
        .run(&["workspace", "restore", "--discard-changes"])
        .status
        .success());
    assert_eq!(fs::read(external.join("new.txt")).unwrap(), b"outside");
}

#[cfg(unix)]
#[test]
fn executable_changes_are_snapshotted_diffed_and_restored_after_deletion() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    let first = repo.change();
    let script = repo.0.join("script.sh");
    fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    let executable = repo.change();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(repo.ok(&["diff", "workspace"]).contains("script.sh"));
    repo.ok(&["snapshot"]);
    repo.ok(&["workspace", "switch", &first]);
    assert_eq!(
        fs::metadata(&script).unwrap().permissions().mode() & 0o111,
        0
    );
    repo.ok(&["workspace", "switch", &executable]);
    assert_ne!(
        fs::metadata(&script).unwrap().permissions().mode() & 0o111,
        0
    );
    fs::remove_file(&script).unwrap();
    repo.ok(&["workspace", "restore", "--discard-changes"]);
    assert_ne!(
        fs::metadata(&script).unwrap().permissions().mode() & 0o111,
        0
    );
    assert!(Command::new(&script).status().unwrap().success());
    fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!repo.run(&["workspace", "switch", &first]).status.success());
}

#[cfg(unix)]
#[test]
fn restore_preserves_private_read_write_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    repo.change();
    let path = repo.0.join("private.txt");
    fs::write(&path, "saved").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    repo.ok(&["snapshot"]);
    fs::write(&path, "edited").unwrap();
    repo.ok(&["workspace", "restore", "--discard-changes"]);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
