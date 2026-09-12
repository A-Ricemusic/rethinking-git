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
        let repo = Self(std::env::temp_dir().join(format!("rgit-metadata-{}", Uuid::new_v4())));
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

fn snapshot_id(repo: &Repo) -> String {
    let w: Value =
        serde_json::from_slice(&fs::read(repo.0.join(".rgit/workspace.json")).unwrap()).unwrap();
    let c: Value = serde_json::from_slice(
        &fs::read(repo.0.join(format!(
            ".rgit/changes/{}.json",
            w["current_change"].as_str().unwrap()
        )))
        .unwrap(),
    )
    .unwrap();
    c["current_snapshot"].as_str().unwrap().to_owned()
}

#[test]
fn public_change_commands_do_not_disclose_private_snapshot_metadata() {
    let repo = Repo::new();
    let change = repo.change();
    fs::write(repo.0.join("public.txt"), "public file").unwrap();
    repo.ok(&[
        "snapshot",
        "--domain",
        "admin",
        "--message",
        "sensitive-investigation",
    ]);
    let secret = snapshot_id(&repo);
    for command in [
        vec!["change", "list"],
        vec!["change", "show", &change],
        vec!["status"],
        vec!["diff", "workspace"],
        vec!["workspace", "info"],
    ] {
        let output = repo.ok(&command);
        assert!(
            !output.contains(&secret),
            "{command:?} leaked snapshot: {output}"
        );
        assert!(
            !output.contains("sensitive-investigation"),
            "{command:?} leaked message"
        );
        assert!(
            output.contains("restricted"),
            "{command:?} did not redact: {output}"
        );
    }
    assert!(repo
        .ok(&["change", "show", &change, "--as", "admin"])
        .contains("sensitive-investigation"));
}

#[test]
fn public_snapshot_does_not_disclose_restricted_parent_id() {
    let repo = Repo::new();
    repo.change();
    repo.ok(&["snapshot", "--domain", "admin"]);
    let parent = snapshot_id(&repo);
    repo.ok(&["snapshot"]);
    let child = snapshot_id(&repo);
    let output = repo.ok(&["snapshot-info", "show", &child]);
    assert!(
        !output.contains(&parent),
        "private parent disclosed: {output}"
    );
    assert!(output.contains("parent: restricted"));
}

#[test]
fn line_listing_and_diffs_redact_private_integration_ids() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("public.txt"), "content").unwrap();
    repo.ok(&["snapshot", "--domain", "admin"]);
    repo.ok(&["line", "integrate", "main", "--as", "admin"]);
    let line: Value =
        serde_json::from_slice(&fs::read(repo.0.join(".rgit/lines/main.json")).unwrap()).unwrap();
    let head = line["head_snapshot"].as_str().unwrap();
    for command in [
        vec!["line", "list"],
        vec!["diff", "line", "main"],
        vec!["line", "view", "main"],
    ] {
        let output = repo.ok(&command);
        assert!(!output.contains(head), "{command:?} leaked head: {output}");
        assert!(output.contains("restricted"));
    }
}

#[test]
fn snapshot_metadata_does_not_disclose_a_restricted_owning_change() {
    let repo = Repo::new();
    repo.ok(&["actor", "set", "viewer", "--domain", "visible"]);
    repo.ok(&["change", "new", "private", "--domain", "team/private"]);
    repo.ok(&["snapshot", "--domain", "visible"]);
    let snapshot = snapshot_id(&repo);
    let w: Value =
        serde_json::from_slice(&fs::read(repo.0.join(".rgit/workspace.json")).unwrap()).unwrap();
    let change = w["current_change"].as_str().unwrap();
    for command in [
        vec!["snapshot-info", "list", "--as", "viewer"],
        vec!["snapshot-info", "show", &snapshot, "--as", "viewer"],
    ] {
        assert!(
            !repo.ok(&command).contains(change),
            "private change ID leaked"
        );
    }
}
