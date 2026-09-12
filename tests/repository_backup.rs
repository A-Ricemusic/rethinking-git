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
        let repo = Self(std::env::temp_dir().join(format!("rgit-backup-{}", Uuid::new_v4())));
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

fn destination() -> PathBuf {
    std::env::temp_dir().join(format!("rgit-backup-copy-{}", Uuid::new_v4()))
}

#[test]
fn verified_backup_restores_saved_history_and_excludes_unsaved_edits() {
    let source = Repo::new();
    source.change();
    fs::write(source.0.join("saved.txt"), "saved").unwrap();
    source.ok(&["snapshot"]);
    source.ok(&["line", "integrate"]);
    fs::write(source.0.join("saved.txt"), "unsaved").unwrap();
    fs::write(source.0.join("untracked.txt"), "not saved").unwrap();
    let path = destination();
    source.ok(&["repo", "backup", path.to_str().unwrap(), "--as", "admin"]);
    let copy = Repo(path);
    copy.ok(&["repo", "verify", "--as", "admin"]);
    copy.ok(&["workspace", "restore", "--discard-changes", "--as", "admin"]);
    assert_eq!(fs::read(copy.0.join("saved.txt")).unwrap(), b"saved");
    assert!(!copy.0.join("untracked.txt").exists());
    assert_eq!(fs::read(source.0.join("saved.txt")).unwrap(), b"unsaved");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&copy.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

#[test]
fn backup_refuses_existing_destination_without_overwriting_it() {
    let source = Repo::new();
    let path = destination();
    fs::create_dir(&path).unwrap();
    let copy = Repo(path);
    fs::write(copy.0.join("precious.txt"), "keep").unwrap();
    assert!(!source
        .run(&["repo", "backup", copy.0.to_str().unwrap(), "--as", "admin"])
        .status
        .success());
    assert_eq!(fs::read(copy.0.join("precious.txt")).unwrap(), b"keep");
}

#[test]
fn unauthorized_or_corrupt_source_does_not_create_a_backup() {
    let source = Repo::new();
    let path = destination();
    assert!(!source
        .run(&["repo", "backup", path.to_str().unwrap()])
        .status
        .success());
    assert!(!path.exists());
    source.change();
    fs::write(source.0.join("saved.txt"), "saved").unwrap();
    source.ok(&["snapshot"]);
    let blob = fs::read_dir(source.0.join(".rgit/blobs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(blob, "corrupt").unwrap();
    assert!(!source
        .run(&["repo", "backup", path.to_str().unwrap(), "--as", "admin"])
        .status
        .success());
    assert!(!path.exists());
}
