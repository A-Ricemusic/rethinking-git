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
        let repo = Self(std::env::temp_dir().join(format!("rgit-ignore-{}", Uuid::new_v4())));
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
    fn paths(&self, change: &str) -> Vec<String> {
        let change: Value = serde_json::from_slice(
            &fs::read(self.0.join(format!(".rgit/changes/{change}.json"))).unwrap(),
        )
        .unwrap();
        let snapshot: Value = serde_json::from_slice(
            &fs::read(self.0.join(format!(
                ".rgit/snapshots/{}.json",
                change["current_snapshot"].as_str().unwrap()
            )))
            .unwrap(),
        )
        .unwrap();
        snapshot["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap().to_string())
            .collect()
    }
}

#[test]
fn nested_ignore_rules_match_git_for_untracked_files() {
    let repo = Repo::new();
    let change = repo.change();
    fs::write(
        repo.0.join(".gitignore"),
        ".env\n*.log\nignored/\n!ignored/cannot-reinclude.txt\n",
    )
    .unwrap();
    fs::create_dir(repo.0.join("nested")).unwrap();
    fs::create_dir(repo.0.join("ignored")).unwrap();
    fs::write(repo.0.join("nested/.gitignore"), "!keep.log\nlocal.txt\n").unwrap();
    for path in [
        ".env",
        "root.log",
        "keep.txt",
        "nested/keep.log",
        "nested/drop.log",
        "nested/local.txt",
        "ignored/cannot-reinclude.txt",
    ] {
        fs::write(repo.0.join(path), "contents").unwrap();
    }
    let initialized = Command::new("git")
        .current_dir(&repo.0)
        .args(["init", "-q"])
        .output()
        .unwrap();
    assert!(initialized.status.success());
    repo.ok(&["snapshot"]);
    let paths = repo.paths(&change);
    assert!(!paths.contains(&".env".to_string()));
    assert!(paths.contains(&"nested/keep.log".to_string()));
    let output = Command::new("git")
        .current_dir(&repo.0)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let mut expected: Vec<String> = output
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| std::str::from_utf8(p).unwrap().to_string())
        .filter(|p| !p.starts_with(".rgit/"))
        .collect();
    expected.sort();
    assert_eq!(paths, expected);
}

#[test]
fn tracked_files_remain_tracked_when_their_parent_becomes_ignored() {
    let repo = Repo::new();
    let change = repo.change();
    fs::create_dir(repo.0.join("cache")).unwrap();
    fs::write(repo.0.join("cache/tracked.txt"), "old").unwrap();
    repo.ok(&["snapshot"]);
    fs::write(repo.0.join(".gitignore"), "cache/\n").unwrap();
    fs::write(repo.0.join("cache/tracked.txt"), "new").unwrap();
    fs::write(repo.0.join("cache/untracked.txt"), "ignore").unwrap();
    assert!(repo
        .ok(&["diff", "workspace"])
        .contains("cache/tracked.txt"));
    repo.ok(&["snapshot"]);
    let paths = repo.paths(&change);
    assert!(paths.contains(&"cache/tracked.txt".to_string()));
    assert!(!paths.contains(&"cache/untracked.txt".to_string()));
    fs::remove_file(repo.0.join("cache/tracked.txt")).unwrap();
    repo.ok(&["snapshot"]);
    assert!(!repo
        .paths(&change)
        .contains(&"cache/tracked.txt".to_string()));
}

#[cfg(unix)]
#[test]
fn symlinked_ignore_files_are_not_followed() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("rules.txt"), "*.secret\n").unwrap();
    std::os::unix::fs::symlink("rules.txt", repo.0.join(".gitignore")).unwrap();
    assert!(!repo.run(&["snapshot"]).status.success());
    assert_eq!(
        fs::read_dir(repo.0.join(".rgit/snapshots"))
            .unwrap()
            .count(),
        0
    );
}
