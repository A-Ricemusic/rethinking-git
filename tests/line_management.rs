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
        let repo = Self(std::env::temp_dir().join(format!("rgit-lines-{}", Uuid::new_v4())));
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
    fn head(&self, name: &str) -> String {
        let value: Value = serde_json::from_slice(
            &fs::read(self.0.join(format!(".rgit/lines/{name}.json"))).unwrap(),
        )
        .unwrap();
        value["head_snapshot"].as_str().unwrap().to_string()
    }
}

#[test]
fn separate_lines_preserve_saved_history_and_reset_is_guarded_and_reversible() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("file.txt"), "first").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    let first = repo.head("main");
    repo.ok(&["line", "create", "release"]);
    assert_eq!(repo.head("release"), first);
    fs::write(repo.0.join("file.txt"), "second").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate"]);
    let second = repo.head("main");
    assert_ne!(second, first);
    assert_eq!(repo.head("release"), first);
    fs::write(repo.0.join("file.txt"), "unsaved").unwrap();
    assert!(!repo
        .run(&["line", "reset", "--to", &first, "--expected-head", &first])
        .status
        .success());
    assert_eq!(repo.head("main"), second);
    repo.ok(&["line", "reset", "--to", &first, "--expected-head", &second]);
    assert_eq!(repo.head("main"), first);
    assert_eq!(fs::read(repo.0.join("file.txt")).unwrap(), b"unsaved");
    repo.ok(&["line", "reset", "--to", &second, "--expected-head", &first]);
    assert_eq!(repo.head("main"), second);
    assert!(repo.ok(&["op", "log", "--as", "admin"]).contains(&first));
    repo.ok(&["repo", "verify", "--as", "admin"]);
    assert!(!repo.run(&["line", "create", "release"]).status.success());
    assert_eq!(repo.head("release"), first);
}

#[test]
fn line_management_preserves_restricted_source_policy_and_refuses_denied_actors() {
    let repo = Repo::new();
    repo.ok(&["change", "new", "private", "--domain", "admin"]);
    fs::write(repo.0.join("private.txt"), "private").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate", "--as", "admin"]);
    let head = repo.head("main");
    assert!(!repo
        .run(&["line", "create", "private-copy"])
        .status
        .success());
    repo.ok(&["line", "create", "private-copy", "--as", "admin"]);
    let private: Value =
        serde_json::from_slice(&fs::read(repo.0.join(".rgit/lines/private-copy.json")).unwrap())
            .unwrap();
    assert_eq!(private["policy"]["domains"], serde_json::json!(["admin"]));
    assert!(!repo
        .run(&["line", "reset", "--to", &head, "--expected-head", &head])
        .status
        .success());
    repo.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn concurrent_resets_with_the_same_expected_head_have_only_one_winner() {
    let repo = Repo::new();
    repo.change();
    let mut heads = Vec::new();
    for bytes in ["one", "two", "three"] {
        fs::write(repo.0.join("file.txt"), bytes).unwrap();
        repo.ok(&["snapshot"]);
        repo.ok(&["line", "integrate"]);
        heads.push(repo.head("main"));
    }
    let mut children = Vec::new();
    for to in &heads[..2] {
        children.push(
            Command::new(env!("CARGO_BIN_EXE_rgit"))
                .current_dir(&repo.0)
                .args(["line", "reset", "--to", to, "--expected-head", &heads[2]])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    let successes = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap().status.success())
        .filter(|success| *success)
        .count();
    assert_eq!(successes, 1);
    assert!(heads[..2].contains(&repo.head("main")));
    repo.ok(&["repo", "verify", "--as", "admin"]);
}
