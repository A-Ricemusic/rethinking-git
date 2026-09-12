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
        let repo = Self(std::env::temp_dir().join(format!("rgit-export-{}", Uuid::new_v4())));
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

const AUTHOR: &str = "Fixture Author <fixture@example.test>";
fn destination() -> PathBuf {
    std::env::temp_dir().join(format!("rgit-export-destination-{}", Uuid::new_v4()))
}
fn git(repo: &Repo, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .current_dir(&repo.0)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn exports_content_modes_messages_and_merge_parents_to_valid_git() {
    let source = Repo::new();
    source.change();
    fs::write(source.0.join("file.txt"), b"first\0binary\n").unwrap();
    source.ok(&["snapshot", "-m", "first message"]);
    source.ok(&["line", "integrate"]);
    source.change();
    fs::write(source.0.join("file.txt"), b"second\0binary\n").unwrap();
    fs::write(source.0.join("snow 雪.txt"), "unicode path").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(source.0.join("file.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    source.ok(&["snapshot", "-m", "second message"]);
    source.ok(&["line", "integrate"]);
    let line: Value =
        serde_json::from_slice(&fs::read(source.0.join(".rgit/lines/main.json")).unwrap()).unwrap();
    let head: Value = serde_json::from_slice(
        &fs::read(source.0.join(format!(
            ".rgit/snapshots/{}.json",
            line["head_snapshot"].as_str().unwrap()
        )))
        .unwrap(),
    )
    .unwrap();
    let expected_seconds = head["created_at"].as_u64().unwrap() / 1000;
    let path = destination();
    source.ok(&[
        "git",
        "export",
        path.to_str().unwrap(),
        "--author",
        AUTHOR,
        "--as",
        "admin",
    ]);
    let exported = Repo(path);
    git(&exported, &["fsck", "--full", "--strict"]);
    assert_eq!(
        String::from_utf8(git(&exported, &["show", "-s", "--format=%ct", "HEAD"]))
            .unwrap()
            .trim(),
        expected_seconds.to_string()
    );
    assert_eq!(
        git(&exported, &["show", "HEAD:file.txt"]),
        b"second\0binary\n"
    );
    assert_eq!(
        git(&exported, &["show", "HEAD:snow 雪.txt"]),
        b"unicode path"
    );
    assert_eq!(
        String::from_utf8(git(&exported, &["rev-list", "--count", "HEAD"]))
            .unwrap()
            .trim(),
        "4"
    );
    assert_eq!(
        String::from_utf8(git(&exported, &["show", "-s", "--format=%P", "HEAD"]))
            .unwrap()
            .split_whitespace()
            .count(),
        2
    );
    assert!(
        String::from_utf8(git(&exported, &["log", "--format=%B", "HEAD"]))
            .unwrap()
            .contains("second message")
    );
    #[cfg(unix)]
    assert!(
        String::from_utf8(git(&exported, &["ls-tree", "HEAD", "file.txt"]))
            .unwrap()
            .starts_with("100755")
    );
}

#[test]
fn restricted_export_requires_explicit_policy_removal() {
    let source = Repo::new();
    source.change();
    fs::write(source.0.join("secret.txt"), "restricted").unwrap();
    source.ok(&["access", "path", "secret.txt", "--domain", "admin"]);
    source.ok(&["snapshot"]);
    source.ok(&["line", "integrate", "--as", "admin"]);
    let path = destination();
    let args = [
        "git",
        "export",
        path.to_str().unwrap(),
        "--author",
        AUTHOR,
        "--as",
        "admin",
    ];
    assert!(!source.run(&args).status.success());
    assert!(!path.exists());
    let mut allowed = args.to_vec();
    allowed.push("--allow-restricted");
    source.ok(&allowed);
    let exported = Repo(path);
    assert_eq!(git(&exported, &["show", "HEAD:secret.txt"]), b"restricted");
}

#[test]
fn existing_destinations_and_ambient_git_directories_cannot_be_overwritten() {
    let source = Repo::new();
    source.change();
    source.ok(&["snapshot"]);
    source.ok(&["line", "integrate"]);
    let victim = Repo(destination());
    fs::create_dir(&victim.0).unwrap();
    fs::write(victim.0.join("precious.txt"), "keep").unwrap();
    assert!(!source
        .run(&[
            "git",
            "export",
            victim.0.to_str().unwrap(),
            "--author",
            AUTHOR,
            "--as",
            "admin"
        ])
        .status
        .success());
    let path = destination();
    let output = Command::new(env!("CARGO_BIN_EXE_rgit"))
        .current_dir(&source.0)
        .env("GIT_DIR", &victim.0)
        .env("GIT_OBJECT_DIRECTORY", &victim.0)
        .args([
            "git",
            "export",
            path.to_str().unwrap(),
            "--author",
            AUTHOR,
            "--as",
            "admin",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exported = Repo(path);
    git(&exported, &["fsck", "--full", "--strict"]);
    assert_eq!(fs::read(victim.0.join("precious.txt")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(&victim.0).unwrap().count(), 1);
}
