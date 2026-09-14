use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};
use uuid::Uuid;

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let repo = Self(std::env::temp_dir().join(format!("rgit-json-{}", Uuid::new_v4())));
        fs::create_dir(&repo.0).unwrap();
        repo.call(&["init"], true);
        repo
    }
    fn call(&self, args: &[&str], success: bool) -> Value {
        let out = Command::new(env!("CARGO_BIN_EXE_rgit"))
            .args(args)
            .args(["--output", "json"])
            .current_dir(&self.0)
            .output()
            .unwrap();
        assert_eq!(
            out.status.success(),
            success,
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|error| {
            panic!(
                "{args:?}: {error}: {}",
                String::from_utf8_lossy(&out.stdout)
            )
        });
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["ok"], success);
        assert_eq!(value["exit_code"], out.status.code().unwrap());
        assert_eq!(out.stdout.iter().filter(|b| **b == b'\n').count(), 1);
        value
    }
    fn write(&self, text: &str) {
        fs::write(self.0.join("file.txt"), text).unwrap();
    }
    fn start(&self, name: &str) -> String {
        data(
            &self.call(&["workspace", "start", name], true),
            "change_created",
        )["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn data<'a>(value: &'a Value, kind: &str) -> &'a Value {
    &value["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["kind"] == kind)
        .unwrap_or_else(|| panic!("missing {kind}: {value}"))["data"]
}

#[test]
fn workflow_uses_only_public_json_ids_and_preserves_legacy_status() {
    let repo = Repo::new();
    let change = repo.start("first");
    repo.write("first\n");
    let saved = repo.call(&["snapshot"], true);
    let snapshot = data(&saved, "snapshot_created")["id"].as_str().unwrap();
    assert_eq!(
        data(&repo.call(&["change", "show", &change], true), "change")["current_snapshot"],
        snapshot
    );
    assert_eq!(
        data(&repo.call(&["snapshot-info", "list"], true), "snapshot")["id"],
        snapshot
    );
    repo.call(&["line", "integrate"], true);
    repo.write("second\n");
    let patch = repo.call(&["diff", "workspace", "--patch"], true);
    data(&patch, "diff");
    assert!(patch["text"].as_str().unwrap().contains("+second"));
    assert_eq!(patch["text_truncated"], false);
    let refused = repo.call(&["workspace", "start", "must-not-exist"], false);
    assert_eq!(refused["records"], serde_json::json!([]));
    assert_eq!(refused["text"], "");
    assert_eq!(
        data(&repo.call(&["workspace", "info"], true), "workspace")["change_id"],
        change
    );
    let legacy = Command::new(env!("CARGO_BIN_EXE_rgit"))
        .args(["status", "--json"])
        .current_dir(&repo.0)
        .output()
        .unwrap();
    let legacy: Value = serde_json::from_slice(&legacy.stdout).unwrap();
    assert_eq!(data(&repo.call(&["status"], true), "status"), &legacy);
}

#[test]
fn restricted_metadata_is_redacted_in_the_entire_envelope() {
    let repo = Repo::new();
    let change = repo.start("public");
    repo.write("private\n");
    let saved = repo.call(
        &[
            "snapshot",
            "--domain",
            "admin",
            "--message",
            "secret-investigation",
        ],
        true,
    );
    let snapshot = data(&saved, "snapshot_created")["id"].as_str().unwrap();
    for args in [
        vec!["change", "list"],
        vec!["change", "show", &change],
        vec!["snapshot-info", "list"],
        vec!["workspace", "info"],
        vec!["status"],
    ] {
        let value = repo.call(&args, true).to_string();
        assert!(!value.contains(snapshot), "{args:?}: {value}");
        assert!(!value.contains("secret-investigation"), "{args:?}: {value}");
    }
    let refused = repo.call(&["snapshot-info", "show", snapshot], false);
    assert_eq!(refused["error"]["kind"], "unavailable");
    assert!(!refused.to_string().contains(snapshot));
    data(
        &repo.call(&["snapshot-info", "show", snapshot, "--as", "admin"], true),
        "snapshot",
    );
}

#[test]
fn committed_conflicts_remain_machine_readable_on_nonzero_exit() {
    let repo = Repo::new();
    repo.start("base");
    repo.write("base\n");
    repo.call(&["snapshot"], true);
    repo.call(&["line", "integrate"], true);
    let left = repo.start("left");
    repo.write("left\n");
    repo.call(&["snapshot"], true);
    repo.start("right");
    repo.write("right\n");
    repo.call(&["snapshot"], true);
    repo.call(&["line", "integrate"], true);
    repo.call(&["workspace", "switch", &left], true);
    let conflict = repo.call(&["line", "integrate"], false);
    assert_eq!(conflict["error"]["kind"], "conflicts");
    let id = data(&conflict, "conflict")["id"].as_str().unwrap();
    assert_eq!(
        data(&repo.call(&["conflict", "list"], true), "conflict")["id"],
        id
    );
    repo.call(&["conflict", "resolve", id, "--take", "incoming"], true);
    data(&repo.call(&["line", "integrate"], true), "integration");
}

#[test]
fn git_export_returns_a_single_json_document() {
    let repo = Repo::new();
    repo.call(&["identity", "set", "JSON Test", "json@example.test"], true);
    repo.start("export");
    repo.write("saved\n");
    repo.call(&["snapshot"], true);
    repo.call(&["line", "integrate"], true);
    let exported = Repo(repo.0.with_extension("export.git"));
    let destination = &exported.0;
    let outcome = repo.call(
        &[
            "git",
            "export",
            destination.to_str().unwrap(),
            "--as",
            "admin",
        ],
        true,
    );
    data(&outcome, "git_export");
}
