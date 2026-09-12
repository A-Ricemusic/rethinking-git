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
        let repo = Self(std::env::temp_dir().join(format!("rgit-identity-{}", Uuid::new_v4())));
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
    std::env::temp_dir().join(format!("rgit-identity-destination-{}", Uuid::new_v4()))
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
fn snapshots_keep_their_authors_when_configuration_changes_before_export() {
    let repo = Repo::new();
    repo.change();
    repo.ok(&["identity", "set", "Alice", "alice@example.test"]);
    fs::write(repo.0.join("file"), "first").unwrap();
    repo.ok(&["snapshot", "--message", "first"]);
    repo.ok(&["identity", "set", "Bob", "bob@example.test"]);
    fs::write(repo.0.join("file"), "second").unwrap();
    repo.ok(&["snapshot", "--message", "second"]);
    repo.ok(&["line", "integrate"]);
    repo.ok(&["identity", "set", "Carol", "carol@example.test"]);
    let exported = Repo(destination());
    repo.ok(&[
        "git",
        "export",
        exported.0.to_str().unwrap(),
        "--as",
        "admin",
    ]);
    let log =
        String::from_utf8(git(&exported, &["log", "--all", "--format=%s|%an <%ae>"])).unwrap();
    assert!(log
        .lines()
        .any(|line| line == "first|Alice <alice@example.test>"));
    assert!(log
        .lines()
        .any(|line| line == "second|Bob <bob@example.test>"));
    assert!(!log.contains("Carol"));
    repo.ok(&["repo", "verify", "--as", "admin"]);
    let second = Repo(destination());
    repo.ok(&[
        "git",
        "export",
        second.0.to_str().unwrap(),
        "--as",
        "admin",
        "--author",
        AUTHOR,
    ]);
    assert_eq!(
        git(&exported, &["rev-parse", "HEAD"]),
        git(&second, &["rev-parse", "HEAD"])
    );
}

#[test]
fn legacy_snapshots_require_explicit_fallback_and_bad_identity_updates_do_not_publish() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("file"), "legacy").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    repo.ok(&["identity", "set", "Future", "future@example.test"]);
    let before = fs::read(repo.0.join(".rgit/repo.json")).unwrap();
    for (name, email) in [
        ("Bad\nName", "mail@example.test"),
        ("Name", "bad mail@example.test"),
        ("", "mail@example.test"),
    ] {
        assert!(!repo.run(&["identity", "set", name, email]).status.success());
        assert_eq!(fs::read(repo.0.join(".rgit/repo.json")).unwrap(), before);
    }
    let exported = Repo(destination());
    assert!(!repo
        .run(&[
            "git",
            "export",
            exported.0.to_str().unwrap(),
            "--as",
            "admin"
        ])
        .status
        .success());
    assert!(!exported.0.exists());
    repo.ok(&[
        "git",
        "export",
        exported.0.to_str().unwrap(),
        "--as",
        "admin",
        "--author",
        AUTHOR,
    ]);
    let authors = String::from_utf8(git(&exported, &["log", "--format=%an <%ae>"])).unwrap();
    assert!(authors.lines().all(|author| author == AUTHOR));
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn verification_detects_a_recorded_author_that_disagrees_with_bound_git_metadata() {
    let repo = Repo::new();
    repo.change();
    repo.ok(&["identity", "set", "Alice", "alice@example.test"]);
    fs::write(repo.0.join("file"), "saved").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    let exported = Repo(destination());
    repo.ok(&[
        "git",
        "export",
        exported.0.to_str().unwrap(),
        "--as",
        "admin",
    ]);
    let path = fs::read_dir(repo.0.join(".rgit/snapshots"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut snapshot: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let shown = repo.ok(&["snapshot-info", "show", snapshot["id"].as_str().unwrap()]);
    assert!(shown.contains("author: Alice <alice@example.test>"));
    snapshot["author"] = serde_json::json!("Different <different@example.test>");
    fs::write(path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let result = repo.run(&["repo", "verify", "--as", "admin"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Git provenance author"));
}
