use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

struct Trial {
    root: PathBuf,
    primary: PathBuf,
}
impl Trial {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("rgit-worktrees-{}", Uuid::new_v4()));
        let primary = root.join("primary");
        fs::create_dir_all(&primary).unwrap();
        let trial = Self { root, primary };
        call(&trial.primary, &["init"], true);
        call(&trial.primary, &["workspace", "start", "base"], true);
        fs::write(trial.primary.join("file.txt"), "saved\n").unwrap();
        call(&trial.primary, &["snapshot"], true);
        call(&trial.primary, &["line", "integrate"], true);
        trial
    }
    fn add(&self, name: &str) -> (PathBuf, Value) {
        let path = self.root.join(name);
        let result = call(
            &self.primary,
            &["worktree", "add", path.to_str().unwrap(), "--name", name],
            true,
        );
        (path, record(&result, "worktree").clone())
    }
}
impl Drop for Trial {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn call(root: &Path, args: &[&str], success: bool) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_rgit"))
        .current_dir(root)
        .args(args)
        .args(["--output", "json"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn record<'a>(out: &'a Value, kind: &str) -> &'a Value {
    &out["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == kind)
        .unwrap_or_else(|| panic!("missing {kind}: {out}"))["data"]
}

#[test]
fn linked_directories_share_history_and_keep_independent_files_and_changes() {
    let trial = Trial::new();
    let (left, left_info) = trial.add("left");
    let (right, _) = trial.add("right");
    assert!(!left.join(".rgit/blobs").exists());
    assert_eq!(
        fs::read_to_string(left.join("file.txt")).unwrap(),
        "saved\n"
    );
    fs::write(left.join("left.txt"), "left").unwrap();
    fs::write(right.join("right.txt"), "right").unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| call(&left, &["snapshot"], true));
        scope.spawn(|| call(&right, &["snapshot"], true));
    });
    call(&left, &["line", "integrate"], true);
    call(&right, &["line", "integrate"], true);
    let files = call(&trial.primary, &["line", "view"], true).to_string();
    assert!(files.contains("left.txt") && files.contains("right.txt"));
    assert!(!trial.primary.join("left.txt").exists());
    assert!(!left.join("right.txt").exists());
    let refused = call(
        &right,
        &[
            "workspace",
            "switch",
            left_info["change_id"].as_str().unwrap(),
        ],
        false,
    );
    assert!(refused["error"]["message"]
        .as_str()
        .unwrap()
        .contains("already active"));
    call(&right, &["repo", "verify", "--as", "admin"], true);
    let list = call(&left, &["worktree", "list"], true);
    assert_eq!(list["records"].as_array().unwrap().len(), 3);
}

#[test]
fn a_command_in_another_directory_recovers_the_correct_worktree() {
    let trial = Trial::new();
    let (linked, info) = trial.add("linked");
    let journal =
        rusqlite::Connection::open(trial.primary.join(".rgit/command-journal.sqlite3")).unwrap();
    journal
        .execute(
            "INSERT INTO workspace_context VALUES (1, ?1)",
            [info["id"].as_str().unwrap()],
        )
        .unwrap();
    journal
        .execute(
            "INSERT INTO working VALUES ('file.txt', ?1, ?2, 0, 0, 0, 0)",
            rusqlite::params![b"saved\n".as_slice(), b"recovered\n".as_slice()],
        )
        .unwrap();
    drop(journal);
    call(&trial.primary, &["status"], true);
    assert_eq!(
        fs::read_to_string(linked.join("file.txt")).unwrap(),
        "recovered\n"
    );
    assert_eq!(
        fs::read_to_string(trial.primary.join("file.txt")).unwrap(),
        "saved\n"
    );
}

#[test]
fn resume_is_idempotent_and_detach_preserves_dirty_and_untracked_files() {
    let trial = Trial::new();
    let (linked, info) = trial.add("linked");
    fs::write(linked.join("file.txt"), "dirty").unwrap();
    fs::write(linked.join("untracked.txt"), "keep").unwrap();
    let resumed = call(
        &trial.primary,
        &[
            "worktree",
            "add",
            linked.to_str().unwrap(),
            "--name",
            "linked",
            "--resume",
        ],
        true,
    );
    assert_eq!(record(&resumed, "worktree")["change_id"], info["change_id"]);
    call(
        &trial.primary,
        &["worktree", "detach", linked.to_str().unwrap()],
        true,
    );
    call(
        &trial.primary,
        &["worktree", "detach", linked.to_str().unwrap()],
        true,
    );
    assert!(!linked.join(".rgit").exists());
    assert_eq!(
        fs::read_to_string(linked.join("file.txt")).unwrap(),
        "dirty"
    );
    assert_eq!(
        fs::read_to_string(linked.join("untracked.txt")).unwrap(),
        "keep"
    );
    call(
        &trial.primary,
        &["workspace", "switch", info["change_id"].as_str().unwrap()],
        true,
    );
    call(&trial.primary, &["repo", "verify", "--as", "admin"], true);
}

#[test]
fn overlap_and_foreign_destinations_are_refused_without_overwriting_files() {
    let trial = Trial::new();
    for path in [trial.primary.join("nested"), trial.root.clone()] {
        call(
            &trial.primary,
            &["worktree", "add", path.to_str().unwrap(), "--name", "bad"],
            false,
        );
    }
    let foreign = trial.root.join("foreign");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("keep"), "keep").unwrap();
    call(
        &trial.primary,
        &[
            "worktree",
            "add",
            foreign.to_str().unwrap(),
            "--name",
            "bad",
        ],
        false,
    );
    assert_eq!(fs::read_to_string(foreign.join("keep")).unwrap(), "keep");
}

#[test]
fn interrupted_add_resumes_the_pinned_snapshot_after_the_line_advances() {
    let trial = Trial::new();
    let listing = call(&trial.primary, &["line", "view"], true);
    let hash = record(&listing, "file")["hash"].as_str().unwrap();
    let blob = trial.primary.join(".rgit/blobs").join(hash);
    let original = fs::read(&blob).unwrap();
    fs::write(&blob, "corrupt").unwrap();
    let linked = trial.root.join("interrupted");
    call(
        &trial.primary,
        &[
            "worktree",
            "add",
            linked.to_str().unwrap(),
            "--name",
            "task",
        ],
        false,
    );
    assert!(linked.join(".rgit/linked.json").is_file());
    fs::write(&blob, original).unwrap();
    fs::write(trial.primary.join("file.txt"), "advanced\n").unwrap();
    call(&trial.primary, &["snapshot"], true);
    call(&trial.primary, &["line", "integrate"], true);
    call(
        &trial.primary,
        &[
            "worktree",
            "add",
            linked.to_str().unwrap(),
            "--name",
            "task",
            "--resume",
        ],
        true,
    );
    assert_eq!(
        fs::read_to_string(linked.join("file.txt")).unwrap(),
        "saved\n"
    );
    let status = call(&linked, &["status"], true);
    assert_eq!(
        record(&status, "status")["changes"]["modified"],
        serde_json::json!([])
    );
}

#[test]
fn tampered_marker_stops_recovery_before_any_working_file_is_written() {
    let trial = Trial::new();
    let (linked, info) = trial.add("linked");
    let marker = linked.join(".rgit/linked.json");
    let original = fs::read(&marker).unwrap();
    let mut changed: Value = serde_json::from_slice(&original).unwrap();
    changed["repo_id"] = "repo_00000000000000000000000000000000".into();
    fs::write(&marker, serde_json::to_vec(&changed).unwrap()).unwrap();
    let journal =
        rusqlite::Connection::open(trial.primary.join(".rgit/command-journal.sqlite3")).unwrap();
    journal
        .execute(
            "INSERT INTO workspace_context VALUES (1, ?1)",
            [info["id"].as_str().unwrap()],
        )
        .unwrap();
    journal
        .execute(
            "INSERT INTO working VALUES ('file.txt', ?1, ?2, 0, 0, 0, 0)",
            rusqlite::params![b"saved\n".as_slice(), b"recovered\n".as_slice()],
        )
        .unwrap();
    drop(journal);
    call(&trial.primary, &["status"], false);
    assert_eq!(
        fs::read_to_string(linked.join("file.txt")).unwrap(),
        "saved\n"
    );
    assert_eq!(
        fs::read_to_string(trial.primary.join("file.txt")).unwrap(),
        "saved\n"
    );
    fs::write(marker, original).unwrap();
    call(&trial.primary, &["status"], true);
    assert_eq!(
        fs::read_to_string(linked.join("file.txt")).unwrap(),
        "recovered\n"
    );
}

#[test]
fn a_backup_from_a_linked_directory_restores_independently() {
    let trial = Trial::new();
    let (linked, _) = trial.add("linked");
    fs::write(linked.join("linked.txt"), "saved linked work").unwrap();
    call(&linked, &["snapshot"], true);
    let backup = trial.root.join("backup");
    call(
        &linked,
        &["repo", "backup", backup.to_str().unwrap(), "--as", "admin"],
        true,
    );
    assert!(!backup.join(".rgit/worktrees.json").exists());
    fs::rename(&trial.primary, trial.root.join("offline-primary")).unwrap();
    call(
        &backup,
        &["workspace", "restore", "--discard-changes", "--as", "admin"],
        true,
    );
    assert_eq!(
        fs::read_to_string(backup.join("linked.txt")).unwrap(),
        "saved linked work"
    );
    call(&backup, &["repo", "verify", "--as", "admin"], true);
}

#[test]
fn unavailable_worktrees_without_pending_writes_do_not_block_other_commands() {
    let trial = Trial::new();
    let (linked, info) = trial.add("linked");
    fs::rename(&linked, trial.root.join("offline")).unwrap();
    call(&trial.primary, &["snapshot"], true);
    let list = call(&trial.primary, &["worktree", "list"], true);
    let entry = list["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["data"]["id"] == info["id"])
        .unwrap();
    assert_eq!(entry["data"]["available"], false);
    call(
        &trial.primary,
        &["workspace", "switch", info["change_id"].as_str().unwrap()],
        false,
    );
}
