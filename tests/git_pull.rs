use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use uuid::Uuid;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let value = Self(std::env::temp_dir().join(format!("rgit-pull-{}", Uuid::new_v4())));
        fs::create_dir(&value.0).unwrap();
        value
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rgit"))
            .args(args)
            .current_dir(&self.0)
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
    fn json(&self, key: &str) -> Value {
        serde_json::from_slice(&fs::read(self.0.join(".rgit").join(key)).unwrap()).unwrap()
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    root: Directory,
    writer: PathBuf,
    client: Directory,
    remote: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = Directory::new();
        let writer = root.0.join("writer");
        fs::create_dir(&writer).unwrap();
        let remote = root.0.join("remote.git");
        let value = Self {
            root,
            writer,
            client: Directory::new(),
            remote,
        };
        value.git(&["init", "--initial-branch=main"]);
        value.git(&["config", "user.name", "Fixture Author"]);
        value.git(&["config", "user.email", "fixture@example.invalid"]);
        value.git(&["config", "core.autocrlf", "false"]);
        fs::write(value.writer.join("file"), "base\n").unwrap();
        value.commit("base");
        value.git(&[
            "clone",
            "--bare",
            value.writer.to_str().unwrap(),
            value.remote.to_str().unwrap(),
        ]);
        // Clone requires an absent destination; Directory owns cleanup afterward.
        fs::remove_dir(&value.client.0).unwrap();
        value.root.ok(&[
            "git",
            "clone",
            value.remote.to_str().unwrap(),
            value.client.0.to_str().unwrap(),
            "--domain",
            "public",
        ]);
        value
    }
    fn git(&self, args: &[&str]) {
        let result = Command::new("git")
            .args(args)
            .current_dir(&self.writer)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    fn commit(&self, message: &str) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "-m", message]);
    }
    fn publish(&self) {
        self.git(&["push", self.remote.to_str().unwrap(), "main"]);
    }
    fn pull(&self) -> Output {
        self.client.run(&[
            "git",
            "pull",
            self.remote.to_str().unwrap(),
            "--as",
            "admin",
        ])
    }
    fn records(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut result = Vec::new();
        for directory in ["changes", "lines", "snapshots", "operations"] {
            for entry in fs::read_dir(self.client.0.join(".rgit").join(directory)).unwrap() {
                let path = entry.unwrap().path();
                result.push((path.clone(), fs::read(path).unwrap()));
            }
        }
        let workspace = self.client.0.join(".rgit/workspace.json");
        result.push((workspace.clone(), fs::read(workspace).unwrap()));
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }
}

#[test]
fn pull_materializes_a_fast_forward_and_starts_a_change_at_the_exact_tip() {
    let fixture = Fixture::new();
    fs::write(fixture.writer.join("file"), "updated\n").unwrap();
    fs::create_dir(fixture.writer.join("nested")).unwrap();
    fs::write(fixture.writer.join("nested/added"), "new\n").unwrap();
    fixture.commit("update");
    fixture.publish();
    fs::write(fixture.client.0.join("untracked"), "keep").unwrap();
    let previous = fixture.client.json("workspace.json");
    let result = fixture.pull();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("working files are unchanged"));
    assert_eq!(
        fs::read(fixture.client.0.join("file")).unwrap(),
        b"updated\n"
    );
    assert_eq!(
        fs::read(fixture.client.0.join("nested/added")).unwrap(),
        b"new\n"
    );
    assert_eq!(
        fs::read(fixture.client.0.join("untracked")).unwrap(),
        b"keep"
    );
    let workspace = fixture.client.json("workspace.json");
    assert_ne!(workspace["current_change"], previous["current_change"]);
    let change = fixture.client.json(&format!(
        "changes/{}.json",
        workspace["current_change"].as_str().unwrap()
    ));
    assert!(change["current_snapshot"].is_null());
    assert_eq!(
        change["base_snapshot"],
        fixture.client.json("lines/main.json")["head_snapshot"]
    );
    assert_eq!(workspace["mode_snapshot"], change["base_snapshot"]);
    assert_eq!(change["policy"]["domains"], serde_json::json!(["public"]));
    fixture.client.ok(&["repo", "verify", "--as", "admin"]);
    fs::write(fixture.client.0.join("file"), "later dirty work").unwrap();
    assert!(fixture.pull().status.success());
    assert_eq!(fixture.client.json("workspace.json"), workspace);
    assert_eq!(
        fs::read(fixture.client.0.join("file")).unwrap(),
        b"later dirty work"
    );
}

#[test]
fn dirty_untracked_and_saved_work_refusals_preserve_all_saved_records() {
    for scenario in ["dirty", "untracked", "saved", "diverged"] {
        let fixture = Fixture::new();
        if scenario == "untracked" {
            fs::write(fixture.writer.join("collision"), "remote").unwrap();
            fs::write(fixture.client.0.join("collision"), "local").unwrap();
        } else {
            fs::write(fixture.client.0.join("file"), "local work").unwrap();
        }
        if matches!(scenario, "saved" | "diverged") {
            fixture.client.ok(&["change", "new", "local"]);
            fixture.client.ok(&["snapshot"]);
            if scenario == "diverged" {
                fixture.client.ok(&["line", "integrate"]);
            }
        }
        fs::write(fixture.writer.join("file"), "remote update").unwrap();
        fixture.commit("remote update");
        fixture.publish();
        let before = fixture.records();
        let result = fixture.pull();
        assert!(!result.status.success(), "{scenario}");
        assert!(
            result.stdout.is_empty(),
            "{scenario}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert_eq!(fixture.records(), before, "{scenario}");
        let path = if scenario == "untracked" {
            "collision"
        } else {
            "file"
        };
        let expected = if scenario == "untracked" {
            "local"
        } else {
            "local work"
        };
        assert_eq!(
            fs::read(fixture.client.0.join(path)).unwrap(),
            expected.as_bytes()
        );
        fixture.client.ok(&["repo", "verify", "--as", "admin"]);
    }
}

#[test]
fn pull_handles_file_directory_transitions_and_refuses_the_wrong_workspace_target() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.writer.join("file")).unwrap();
    fs::create_dir(fixture.writer.join("file")).unwrap();
    fs::write(fixture.writer.join("file/child"), "nested").unwrap();
    fixture.commit("file to directory");
    fixture.publish();
    assert!(fixture.pull().status.success());
    assert_eq!(
        fs::read(fixture.client.0.join("file/child")).unwrap(),
        b"nested"
    );
    fixture.client.ok(&["line", "create", "other"]);
    fixture.client.ok(&["change", "retarget", "other"]);
    let before = fixture.records();
    let result = fixture.pull();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("targets another line"));
    assert_eq!(fixture.records(), before);
}

#[test]
fn pull_uses_the_saved_tip_even_when_its_owning_change_has_newer_work() {
    let fixture = Fixture::new();
    let original = fixture.client.json("workspace.json")["current_change"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture.client.ok(&[
        "identity",
        "set",
        "Fixture Author",
        "fixture@example.invalid",
    ]);
    fixture.client.ok(&["line", "create", "publish"]);
    fixture
        .client
        .ok(&["change", "new", "published", "--target", "publish"]);
    fs::write(fixture.client.0.join("file"), "published content").unwrap();
    fixture.client.ok(&["snapshot"]);
    fixture.client.ok(&["line", "integrate", "publish"]);
    fixture.client.ok(&[
        "git",
        "push",
        fixture.remote.to_str().unwrap(),
        "--line",
        "publish",
        "--branch",
        "main",
        "--as",
        "admin",
    ]);
    let published = fixture.client.json("lines/publish.json")["head_snapshot"].clone();
    fs::write(fixture.client.0.join("file"), "later unintegrated work").unwrap();
    fixture.client.ok(&["snapshot"]);
    fixture.client.ok(&["workspace", "switch", &original]);
    let result = fixture.pull();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        fixture.client.json("lines/main.json")["head_snapshot"],
        published
    );
    assert_eq!(
        fs::read(fixture.client.0.join("file")).unwrap(),
        b"published content"
    );
    fixture.client.ok(&["repo", "verify", "--as", "admin"]);
}
