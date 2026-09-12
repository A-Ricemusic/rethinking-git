use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
use uuid::Uuid;

struct Repo(PathBuf);
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl Repo {
    fn new() -> Self {
        let repo = Self(std::env::temp_dir().join(format!("rgit-patch-{}", Uuid::new_v4())));
        fs::create_dir(&repo.0).unwrap();
        repo.ok(&["init"]);
        repo.ok(&["change", "new", "patches"]);
        repo
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rgit"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let result = self.run(args);
        assert!(
            result.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    }
    fn snapshot(&self) -> String {
        let result = self.ok(&["snapshot"]);
        result
            .split_whitespace()
            .find(|word| word.starts_with("snap_"))
            .unwrap()
            .to_string()
    }
}

fn apply(root: &Path, patch: &str) {
    let mut child = Command::new("git")
        .args(["-c", "core.autocrlf=false", "apply", "-"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{patch}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn workspace_and_snapshot_hunks_reproduce_added_modified_deleted_and_unterminated_text() {
    let repo = Repo::new();
    fs::write(repo.0.join("modified.txt"), "one\ntwo\nthree\n").unwrap();
    fs::write(repo.0.join("deleted.txt"), "remove\n").unwrap();
    fs::write(repo.0.join("no-newline.txt"), "old").unwrap();
    fs::write(repo.0.join("empty-delete"), b"").unwrap();
    fs::write(repo.0.join("crlf.txt"), b"one\r\ntwo\r\n").unwrap();
    let before = repo.snapshot();
    fs::write(repo.0.join("modified.txt"), "one\nTWO\nthree\n").unwrap();
    fs::remove_file(repo.0.join("deleted.txt")).unwrap();
    fs::write(repo.0.join("new file.txt"), "added\n").unwrap();
    fs::write(repo.0.join("no-newline.txt"), "new").unwrap();
    fs::remove_file(repo.0.join("empty-delete")).unwrap();
    fs::write(repo.0.join("empty-add"), b"").unwrap();
    fs::write(repo.0.join("crlf.txt"), b"one\r\nTWO\r\n").unwrap();
    let patch = repo.ok(&["diff", "workspace", "--patch"]);
    assert!(patch.contains("@@"));
    assert!(patch.contains("-two\n+TWO\n"));
    assert!(patch.contains("\\ No newline at end of file"));
    let after = repo.snapshot();
    assert_eq!(
        patch,
        repo.ok(&["diff", "snapshot", &before, &after, "--patch"])
    );
    repo.ok(&[
        "workspace",
        "restore",
        "--from",
        &before,
        "--discard-changes",
    ]);
    apply(&repo.0, &patch);
    assert_eq!(
        fs::read(repo.0.join("modified.txt")).unwrap(),
        b"one\nTWO\nthree\n"
    );
    assert_eq!(fs::read(repo.0.join("new file.txt")).unwrap(), b"added\n");
    assert_eq!(fs::read(repo.0.join("no-newline.txt")).unwrap(), b"new");
    assert!(!repo.0.join("deleted.txt").exists());
    assert!(!repo.0.join("empty-delete").exists());
    assert_eq!(fs::read(repo.0.join("empty-add")).unwrap(), b"");
    assert_eq!(
        fs::read(repo.0.join("crlf.txt")).unwrap(),
        b"one\r\nTWO\r\n"
    );
    repo.ok(&["line", "integrate", "--as", "admin"]);
    assert!(repo
        .ok(&["diff", "line", "--patch", "--as", "admin"])
        .contains("diff --git"));
}

#[test]
fn patch_filters_hidden_paths_and_never_prints_binary_or_terminal_control_bytes() {
    let repo = Repo::new();
    repo.ok(&["access", "path", "secret.txt", "--domain", "admin"]);
    fs::write(repo.0.join("secret.txt"), "hidden-old").unwrap();
    fs::write(repo.0.join("visible.txt"), "old\n").unwrap();
    repo.snapshot();
    fs::write(repo.0.join("secret.txt"), "hidden-new").unwrap();
    fs::write(repo.0.join("visible.txt"), "new\n").unwrap();
    fs::write(repo.0.join("binary.bin"), [0, 255, 27, 91, 109]).unwrap();
    fs::write(repo.0.join("escape.txt"), b"\x1b[31mterminal").unwrap();
    fs::write(repo.0.join("unicode-control.txt"), "\u{009b}terminal").unwrap();
    fs::write(repo.0.join("large.txt"), vec![b'x'; 1_048_577]).unwrap();
    let patch = repo.ok(&["diff", "workspace", "--patch"]);
    assert!(patch.contains("-old\n+new\n"));
    assert!(patch.contains("# hidden: 1 restricted changed path"));
    assert!(patch.contains("binary or non-text"));
    assert!(patch.contains("exceeds 1 MiB"));
    assert!(!patch.contains("secret.txt"));
    assert!(!patch.contains("hidden-old"));
    assert!(!patch.contains("hidden-new"));
    assert!(!patch.contains('\u{009b}'));
    assert!(!patch.contains('\x1b'));
    assert!(!patch.contains('\0'));
    assert!(repo
        .ok(&["diff", "workspace", "--patch", "--as", "admin"])
        .contains("hidden-new"));
}

#[test]
fn corrupt_saved_content_fails_before_a_text_hunk_is_printed() {
    let repo = Repo::new();
    fs::write(repo.0.join("file"), "before\n").unwrap();
    let snapshot = repo.snapshot();
    let value: Value = serde_json::from_slice(
        &fs::read(repo.0.join(format!(".rgit/snapshots/{snapshot}.json"))).unwrap(),
    )
    .unwrap();
    fs::write(
        repo.0
            .join(".rgit/blobs")
            .join(value["files"][0]["hash"].as_str().unwrap()),
        "corrupt",
    )
    .unwrap();
    fs::write(repo.0.join("file"), "after\n").unwrap();
    let output = repo.run(&["diff", "workspace", "--patch"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn unusual_paths_and_mode_only_edits_are_represented_without_external_git() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    let name = "line\nbreak\"back\\slash";
    fs::write(repo.0.join(name), "before\n").unwrap();
    fs::write(repo.0.join("script"), "same\n").unwrap();
    fs::set_permissions(repo.0.join("script"), fs::Permissions::from_mode(0o644)).unwrap();
    repo.snapshot();
    fs::write(repo.0.join(name), "after\n").unwrap();
    fs::set_permissions(repo.0.join("script"), fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rgit"))
        .current_dir(&repo.0)
        .args(["diff", "workspace", "--patch"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(output.status.success());
    let patch = String::from_utf8(output.stdout).unwrap();
    assert!(patch.contains("old mode 100644\nnew mode 100755"));
    assert!(patch.contains("line\\nbreak\\\"back\\\\slash"));
    repo.ok(&["workspace", "restore", "--discard-changes"]);
    apply(&repo.0, &patch);
    assert_eq!(fs::read(repo.0.join(name)).unwrap(), b"after\n");
    assert_ne!(
        fs::metadata(repo.0.join("script"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
}

#[cfg(unix)]
#[test]
fn symlink_patch_reads_target_text_without_following_the_referent() {
    let repo = Repo::new();
    let outside = Repo::new();
    fs::write(outside.0.join("target"), "never-read-this-referent").unwrap();
    std::os::unix::fs::symlink("old-target", repo.0.join("link")).unwrap();
    repo.snapshot();
    fs::remove_file(repo.0.join("link")).unwrap();
    std::os::unix::fs::symlink(outside.0.join("target"), repo.0.join("link")).unwrap();
    let patch = repo.ok(&["diff", "workspace", "--patch"]);
    assert!(patch.contains("-old-target"));
    assert!(!patch.contains("never-read-this-referent"));
    assert_eq!(
        fs::read(outside.0.join("target")).unwrap(),
        b"never-read-this-referent"
    );
}

#[test]
fn policy_only_changes_are_visible_to_admin_and_private_snapshots_are_refused() {
    let repo = Repo::new();
    fs::write(repo.0.join("file"), "same content\n").unwrap();
    let public = repo.snapshot();
    repo.ok(&["access", "path", "file", "--domain", "admin"]);
    let patch = repo.ok(&["diff", "workspace", "--patch", "--as", "admin"]);
    assert!(patch.contains("access policy changed"));
    assert!(!patch.contains("@@"));
    let private = repo.ok(&["snapshot", "--domain", "admin"]);
    let private = private
        .split_whitespace()
        .find(|word| word.starts_with("snap_"))
        .unwrap();
    let output = repo.run(&["diff", "snapshot", &public, private, "--patch"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}
