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

#[cfg(unix)]
#[test]
fn links_are_replaced_without_touching_referents_and_type_changes_are_dirty() {
    use std::os::unix::fs::symlink;
    let repo = Repo::new();
    let outside = Repo::new();
    fs::write(outside.0.join("valuable"), "preserve").unwrap();
    repo.change();
    let target = outside.0.join("valuable");
    symlink(&target, repo.0.join("link")).unwrap();
    repo.ok(&["snapshot"]);
    fs::remove_file(repo.0.join("link")).unwrap();
    fs::write(repo.0.join("link"), target.as_os_str().as_encoded_bytes()).unwrap();
    assert!(!repo
        .run(&["workspace", "restore", "--as", "admin"])
        .status
        .success());
    repo.ok(&["workspace", "restore", "--discard-changes", "--as", "admin"]);
    assert_eq!(fs::read_link(repo.0.join("link")).unwrap(), target);
    assert_eq!(
        fs::read_to_string(outside.0.join("valuable")).unwrap(),
        "preserve"
    );
    // Save a regular file, then restore it over a link with identical target bytes.
    fs::remove_file(repo.0.join("link")).unwrap();
    fs::write(repo.0.join("link"), "other-target").unwrap();
    repo.ok(&["snapshot"]);
    fs::remove_file(repo.0.join("link")).unwrap();
    symlink("other-target", repo.0.join("link")).unwrap();
    repo.ok(&["workspace", "restore", "--discard-changes", "--as", "admin"]);
    assert!(!fs::symlink_metadata(repo.0.join("link"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(repo.0.join("link")).unwrap(), b"other-target");
}

fn git_object(repo: &std::path::Path, args: &[&str], input: &[u8]) -> String {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn import_collision_fixture(repo: &Repo, directories: bool) {
    let source = repo.0.join("fixture.git");
    fs::create_dir(&source).unwrap();
    git_object(&source, &["init", "--bare"], b"");
    let a = git_object(&source, &["hash-object", "-w", "--stdin"], b"first");
    let b = git_object(&source, &["hash-object", "-w", "--stdin"], b"second");
    let tree = if directories {
        let a = git_object(
            &source,
            &["mktree"],
            format!("100644 blob {a}\ta\n").as_bytes(),
        );
        let b = git_object(
            &source,
            &["mktree"],
            format!("100644 blob {b}\tb\n").as_bytes(),
        );
        git_object(
            &source,
            &["mktree"],
            format!("040000 tree {a}\tSrc\n040000 tree {b}\tsrc\n").as_bytes(),
        )
    } else {
        git_object(
            &source,
            &["mktree"],
            format!("100644 blob {a}\tREADME\n100644 blob {b}\treadme\n").as_bytes(),
        )
    };
    let commit = git_object(
        &source,
        &["commit-tree", &tree, "-m", "case collision"],
        b"",
    );
    git_object(&source, &["update-ref", "refs/heads/main", &commit], b"");
    repo.ok(&[
        "git",
        "import",
        source.to_str().unwrap(),
        "--revision",
        "main",
        "--domain",
        "public",
        "--as",
        "admin",
    ]);
    fs::remove_dir_all(&source).unwrap();
}

#[test]
fn aliased_git_paths_are_rejected_before_committing_checkout_on_every_platform() {
    for directories in [false, true] {
        let repo = Repo::new();
        import_collision_fixture(&repo, directories);
        let workspace = fs::read(repo.0.join(".rgit/workspace.json")).unwrap();
        let result = repo.run(&["workspace", "restore", "--discard-changes", "--as", "admin"]);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("alias"));
        assert_eq!(
            fs::read(repo.0.join(".rgit/workspace.json")).unwrap(),
            workspace
        );
        assert_eq!(
            fs::read_dir(&repo.0).unwrap().count(),
            1,
            "checkout published working files"
        );
        let db = rusqlite::Connection::open(repo.0.join(".rgit/command-journal.sqlite3")).unwrap();
        for table in ["pending", "working"] {
            let count: i64 = db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "refused checkout committed recovery rows");
        }
        drop(db);
        // Saved Git data remains verifiable/exportable even when not materializable.
        repo.ok(&["repo", "verify", "--as", "admin"]);
    }
}

#[test]
fn untracked_directory_alias_is_preserved_before_any_checkout_write() {
    let repo = Repo::new();
    let first = repo.change();
    repo.ok(&["snapshot"]);
    fs::create_dir(repo.0.join("src")).unwrap();
    fs::write(repo.0.join("src/tracked"), "saved").unwrap();
    let second = repo.change();
    repo.ok(&["snapshot"]);
    repo.ok(&["workspace", "switch", &first]);
    fs::remove_dir(repo.0.join("src")).unwrap();
    fs::create_dir(repo.0.join("SRC")).unwrap();
    fs::write(repo.0.join("SRC/untracked"), "keep").unwrap();
    let result = repo.run(&["workspace", "switch", &second]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("alias"));
    assert_eq!(repo.current(), first);
    assert_eq!(fs::read(repo.0.join("SRC/untracked")).unwrap(), b"keep");
    assert!(!repo.0.join("SRC/tracked").exists());
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn case_only_switch_is_refused_without_losing_the_saved_or_working_version() {
    let repo = Repo::new();
    let first = repo.change();
    fs::write(repo.0.join("README"), "first").unwrap();
    repo.ok(&["snapshot"]);
    let second = repo.change();
    fs::rename(repo.0.join("README"), repo.0.join("temporary-name")).unwrap();
    fs::rename(repo.0.join("temporary-name"), repo.0.join("readme")).unwrap();
    fs::write(repo.0.join("readme"), "second").unwrap();
    repo.ok(&["snapshot"]);
    let result = repo.run(&["workspace", "switch", &first]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("alias"));
    assert_eq!(repo.current(), second);
    assert_eq!(fs::read(repo.0.join("readme")).unwrap(), b"second");
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn switches_file_and_directory_shapes_in_both_directions_without_metadata_edits() {
    let repo = Repo::new();
    let first = repo.change();
    fs::write(repo.0.join("shape"), "file version").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    let second = repo.change();
    fs::remove_file(repo.0.join("shape")).unwrap();
    fs::create_dir_all(repo.0.join("shape/nested")).unwrap();
    fs::write(repo.0.join("shape/nested/one"), "one").unwrap();
    fs::write(repo.0.join("shape/two"), "two").unwrap();
    repo.ok(&["snapshot"]);
    for _ in 0..2 {
        repo.ok(&["workspace", "switch", &first]);
        assert_eq!(fs::read(repo.0.join("shape")).unwrap(), b"file version");
        repo.ok(&["workspace", "switch", &second]);
        assert_eq!(fs::read(repo.0.join("shape/nested/one")).unwrap(), b"one");
        assert_eq!(fs::read(repo.0.join("shape/two")).unwrap(), b"two");
    }
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn replacing_a_directory_refuses_untracked_files_even_with_discard() {
    let repo = Repo::new();
    let first = repo.change();
    fs::write(repo.0.join("shape"), "saved file").unwrap();
    let snapshot = repo.ok(&["snapshot"]);
    let snapshot = snapshot
        .split_whitespace()
        .find(|word| word.starts_with("snap_"))
        .unwrap();
    let second = repo.change();
    fs::remove_file(repo.0.join("shape")).unwrap();
    fs::create_dir(repo.0.join("shape")).unwrap();
    fs::write(repo.0.join("shape/tracked"), "saved child").unwrap();
    repo.ok(&["snapshot"]);
    fs::write(repo.0.join("shape/untracked"), "keep me").unwrap();
    for command in [
        vec!["workspace", "switch", &first],
        vec![
            "workspace",
            "restore",
            "--from",
            snapshot,
            "--discard-changes",
        ],
    ] {
        let output = repo.run(&command);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("untracked"));
        assert_eq!(repo.current(), second);
        assert_eq!(
            fs::read(repo.0.join("shape/untracked")).unwrap(),
            b"keep me"
        );
        assert_eq!(
            fs::read(repo.0.join("shape/tracked")).unwrap(),
            b"saved child"
        );
    }
    fs::remove_file(repo.0.join("shape/untracked")).unwrap();
    fs::create_dir(repo.0.join("shape/.git")).unwrap();
    assert!(!repo.run(&["workspace", "switch", &first]).status.success());
    assert!(repo.0.join("shape/.git").is_dir());
}

#[cfg(unix)]
#[test]
fn symlink_and_directory_transitions_preserve_external_referents() {
    let repo = Repo::new();
    let first = repo.change();
    fs::write(repo.0.join("outside"), "referent").unwrap();
    std::os::unix::fs::symlink("outside", repo.0.join("shape")).unwrap();
    repo.ok(&["snapshot"]);
    let second = repo.change();
    fs::remove_file(repo.0.join("shape")).unwrap();
    fs::create_dir(repo.0.join("shape")).unwrap();
    fs::write(repo.0.join("shape/inside"), "child").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["workspace", "switch", &first]);
    assert_eq!(
        fs::read_link(repo.0.join("shape")).unwrap(),
        PathBuf::from("outside")
    );
    repo.ok(&["workspace", "switch", &second]);
    assert_eq!(fs::read(repo.0.join("shape/inside")).unwrap(), b"child");
    assert_eq!(fs::read(repo.0.join("outside")).unwrap(), b"referent");
}

#[test]
fn repeated_shape_restore_uses_materialized_paths_without_extra_snapshots() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("shape"), "flat").unwrap();
    repo.ok(&["snapshot"]);
    let flat = current_snapshot_id(&repo);
    repo.ok(&["line", "integrate"]);
    fs::remove_file(repo.0.join("shape")).unwrap();
    fs::create_dir(repo.0.join("shape")).unwrap();
    fs::write(repo.0.join("shape/nested.txt"), "nested").unwrap();
    repo.ok(&["snapshot"]);
    let nested = current_snapshot_id(&repo);
    let snapshot_count = fs::read_dir(repo.0.join(".rgit/snapshots"))
        .unwrap()
        .count();
    for target in [&flat, &nested, &flat, &nested] {
        repo.ok(&[
            "workspace",
            "restore",
            "--from",
            target,
            "--discard-changes",
        ]);
    }
    assert_eq!(
        fs::read_to_string(repo.0.join("shape/nested.txt")).unwrap(),
        "nested"
    );
    assert_eq!(
        fs::read_dir(repo.0.join(".rgit/snapshots"))
            .unwrap()
            .count(),
        snapshot_count
    );
    // A metadata-only change creation must not forget which paths were materialized.
    repo.ok(&["change", "new", "flat-base"]);
    repo.ok(&["workspace", "restore", "--discard-changes"]);
    assert_eq!(fs::read_to_string(repo.0.join("shape")).unwrap(), "flat");
    assert_eq!(
        fs::read_dir(repo.0.join(".rgit/snapshots"))
            .unwrap()
            .count(),
        snapshot_count
    );
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

fn current_snapshot_id(repo: &Repo) -> String {
    let change: Value = serde_json::from_slice(
        &fs::read(
            repo.0
                .join(format!(".rgit/changes/{}.json", repo.current())),
        )
        .unwrap(),
    )
    .unwrap();
    change["current_snapshot"].as_str().unwrap().to_string()
}

#[test]
fn start_checks_out_line_atomically_and_refuses_dirty_work() {
    let repo = Repo::new();
    let (first, _) = repo.branches();
    repo.ok(&["workspace", "start", "fresh"]);
    assert_eq!(fs::read(repo.0.join("file.txt")).unwrap(), b"first");
    assert!(!repo.0.join("nested/new.txt").exists());
    assert_ne!(repo.current(), first);
    let before = fs::read(repo.0.join(".rgit/workspace.json")).unwrap();
    let count = fs::read_dir(repo.0.join(".rgit/changes")).unwrap().count();
    fs::write(repo.0.join("file.txt"), "unsaved").unwrap();
    assert!(!repo
        .run(&["workspace", "start", "must-not-exist"])
        .status
        .success());
    assert_eq!(
        fs::read(repo.0.join(".rgit/workspace.json")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_dir(repo.0.join(".rgit/changes")).unwrap().count(),
        count
    );
    assert_eq!(fs::read(repo.0.join("file.txt")).unwrap(), b"unsaved");
}
