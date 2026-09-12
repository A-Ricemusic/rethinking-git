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
        let repo = Self(std::env::temp_dir().join(format!("rgit-scan-{}", Uuid::new_v4())));
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

fn current_snapshot(repo: &Repo) -> Value {
    let workspace: Value =
        serde_json::from_slice(&fs::read(repo.0.join(".rgit/workspace.json")).unwrap()).unwrap();
    let change: Value = serde_json::from_slice(
        &fs::read(repo.0.join(format!(
            ".rgit/changes/{}.json",
            workspace["current_change"].as_str().unwrap()
        )))
        .unwrap(),
    )
    .unwrap();
    serde_json::from_slice(
        &fs::read(repo.0.join(format!(
            ".rgit/snapshots/{}.json",
            change["current_snapshot"].as_str().unwrap()
        )))
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn leading_spaces_are_preserved_in_paths_and_access_policies() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join(" secret.txt"), "private").unwrap();
    fs::write(repo.0.join("secret.txt"), "public").unwrap();
    repo.ok(&["access", "path", "./ secret.txt", "--domain", "admin"]);
    repo.ok(&["snapshot"]);
    let snapshot = current_snapshot(&repo);
    let files = snapshot["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["path"], " secret.txt");
    assert_eq!(files[0]["policy"]["domains"][0], "admin");
    assert_eq!(files[1]["path"], "secret.txt");
    assert_eq!(files[1]["policy"]["domains"][0], "public");
}

#[cfg(unix)]
#[test]
fn literal_backslashes_do_not_alias_directory_separators() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("a\\b"), "literal filename").unwrap();
    fs::create_dir(repo.0.join("a")).unwrap();
    fs::write(repo.0.join("a/b"), "nested file").unwrap();
    repo.ok(&["access", "path", "a\\b", "--domain", "admin"]);
    repo.ok(&["snapshot"]);
    let snapshot = current_snapshot(&repo);
    let files = snapshot["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_ne!(files[0]["path"], files[1]["path"]);
    assert!(files
        .iter()
        .any(|f| f["path"] == "a\\b" && f["policy"]["domains"][0] == "admin"));
}

#[cfg(unix)]
#[test]
fn symlinks_preserve_targets_without_following_directories_or_dangling_links() {
    use std::os::unix::fs::symlink;
    for target in ["missing", "."] {
        let repo = Repo::new();
        repo.change();
        symlink(target, repo.0.join("link")).unwrap();
        repo.ok(&["snapshot"]);
        repo.ok(&["repo", "verify", "--as", "admin"]);
        fs::remove_file(repo.0.join("link")).unwrap();
        repo.ok(&["workspace", "restore", "--discard-changes", "--as", "admin"]);
        assert_eq!(
            fs::read_link(repo.0.join("link")).unwrap(),
            PathBuf::from(target)
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_filenames_are_refused_instead_of_lossily_rewritten() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join(OsString::from_vec(vec![b'f', 0xff])), "content").unwrap();
    let result = repo.run(&["snapshot"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("non-UTF-8"));
    assert_eq!(
        fs::read_dir(repo.0.join(".rgit/snapshots"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn corrupted_existing_blob_is_not_reused_by_a_new_snapshot() {
    let repo = Repo::new();
    let change = repo.change();
    fs::write(repo.0.join("file.txt"), "original").unwrap();
    repo.ok(&["snapshot"]);
    let snapshot = current_snapshot(&repo);
    let blob = repo
        .0
        .join(".rgit/blobs")
        .join(snapshot["files"][0]["hash"].as_str().unwrap());
    fs::write(&blob, "corrupted").unwrap();
    let change_path = repo.0.join(format!(".rgit/changes/{change}.json"));
    let before = fs::read(&change_path).unwrap();
    let result = repo.run(&["snapshot"]);
    assert!(!result.status.success(), "corrupted blob was reused");
    assert!(String::from_utf8_lossy(&result.stderr).contains("stored blob failed verification"));
    assert_eq!(fs::read(&change_path).unwrap(), before);
    assert_eq!(
        fs::read_dir(repo.0.join(".rgit/snapshots"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn path_policies_must_be_relative_nonempty_paths() {
    let repo = Repo::new();
    for path in ["", ".", "..", "../secret", "/secret"] {
        assert!(
            !repo
                .run(&["access", "path", path, "--domain", "admin"])
                .status
                .success(),
            "accepted {path:?}"
        );
    }
    let policies: Value =
        serde_json::from_slice(&fs::read(repo.0.join(".rgit/path-policies.json")).unwrap())
            .unwrap();
    assert!(policies.as_array().unwrap().is_empty());
}

#[test]
fn large_binary_capture_reuse_and_corruption_preserve_exact_bytes() {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let repo = Repo::new();
    repo.change();
    let chunk: Vec<_> = (0..65_537).map(|i| (i % 251) as u8).collect();
    let mut hash = Sha256::new();
    let mut file = fs::File::create(repo.0.join("large.bin")).unwrap();
    for _ in 0..33 {
        file.write_all(&chunk).unwrap();
        hash.update(&chunk);
    }
    drop(file);
    fs::write(repo.0.join("empty"), []).unwrap();
    repo.ok(&["snapshot"]);
    let first = current_snapshot(&repo);
    repo.ok(&["snapshot"]);
    assert_eq!(first["files"], current_snapshot(&repo)["files"]);
    let blob = repo
        .0
        .join(".rgit/blobs")
        .join(hex::encode(hash.finalize()));
    assert_eq!(
        fs::read(&blob).unwrap(),
        fs::read(repo.0.join("large.bin")).unwrap()
    );
    let names: Vec<_> = fs::read_dir(repo.0.join(".rgit/blobs"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2, "publication temporaries were not removed");
    repo.ok(&["repo", "verify", "--as", "admin"]);
    // A long matching prefix with one extra byte must not be admitted.
    fs::OpenOptions::new()
        .append(true)
        .open(&blob)
        .unwrap()
        .write_all(b"x")
        .unwrap();
    assert!(!repo.run(&["snapshot"]).status.success());
    assert_eq!(fs::read_dir(repo.0.join(".rgit/blobs")).unwrap().count(), 2);
    assert_eq!(
        fs::read_dir(repo.0.join(".rgit/snapshots"))
            .unwrap()
            .count(),
        2
    );
}
