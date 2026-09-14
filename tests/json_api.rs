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
        vec!["status", "--workflow"],
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

#[cfg(target_os = "linux")]
#[test]
fn backup_outcome_handles_non_utf8_destination_without_panicking_after_publication() {
    use std::os::unix::ffi::OsStringExt;
    let repo = Repo::new();
    let destination = Repo(
        repo.0.with_file_name(std::ffi::OsString::from_vec(
            format!("rgit-backup-{}-", Uuid::new_v4())
                .into_bytes()
                .into_iter()
                .chain([0xff])
                .collect(),
        )),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rgit"))
        .current_dir(&repo.0)
        .args(["--output", "json", "repo", "backup"])
        .arg(&destination.0)
        .args(["--as", "admin"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let outcome: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(data(&outcome, "backup")["destination"].is_null());
    assert!(destination.0.join(".rgit/repo.json").is_file());
}

#[test]
fn integration_retry_returns_the_existing_head_without_creating_another_snapshot() {
    let repo = Repo::new();
    let change = repo.start("retry");
    repo.write("saved\n");
    repo.call(&["snapshot"], true);
    let integrated = repo.call(&["line", "integrate"], true);
    let result = data(&integrated, "integration");
    assert_eq!(result["changed"], true);
    let before = repo.call(&["snapshot-info", "list"], true);
    let repeated = repo.call(&["line", "integrate"], true);
    let retry = data(&repeated, "integration");
    assert_eq!(retry["changed"], false);
    assert_eq!(retry["change_id"], change);
    assert_eq!(retry["snapshot_id"], result["snapshot_id"]);
    assert_eq!(repo.call(&["snapshot-info", "list"], true), before);
}

#[test]
fn git_push_success_retry_and_rejection_each_emit_exactly_one_json_outcome() {
    let repo = Repo::new();
    repo.call(
        &["identity", "set", "JSON Agent", "agent@example.test"],
        true,
    );
    repo.start("base");
    repo.write("base\n");
    repo.call(&["snapshot"], true);
    repo.call(&["line", "integrate"], true);
    let remote = Repo(repo.0.with_extension("remote.git"));
    let peer = Repo(repo.0.with_extension("peer"));
    let remote_path = remote.0.to_str().unwrap();
    repo.call(&["git", "export", remote_path, "--as", "admin"], true);
    repo.call(
        &[
            "git",
            "clone",
            remote_path,
            peer.0.to_str().unwrap(),
            "--domain",
            "public",
        ],
        true,
    );
    peer.call(
        &["identity", "set", "Peer Agent", "peer@example.test"],
        true,
    );
    peer.start("peer");
    peer.write("peer\n");
    peer.call(&["snapshot"], true);
    peer.call(&["line", "integrate"], true);
    data(
        &peer.call(&["git", "push", remote_path, "--as", "admin"], true),
        "git_push",
    );
    data(
        &peer.call(&["git", "push", remote_path, "--as", "admin"], true),
        "git_push",
    );
    repo.start("local");
    repo.write("local\n");
    repo.call(&["snapshot"], true);
    repo.call(&["line", "integrate"], true);
    let rejected = repo.call(&["git", "push", remote_path, "--as", "admin"], false);
    assert_eq!(rejected["error"]["kind"], "command_failed");
    assert_eq!(rejected["records"], serde_json::json!([]));
    let text = Command::new(env!("CARGO_BIN_EXE_rgit"))
        .current_dir(&peer.0)
        .args(["git", "push", remote_path, "--as", "admin"])
        .output()
        .unwrap();
    assert!(text.status.success());
    assert!(String::from_utf8_lossy(&text.stdout).contains("refs/heads/main"));
}

#[test]
fn workflow_status_distinguishes_clean_files_from_integrated_saved_work() {
    let repo = Repo::new();
    assert_eq!(
        data(&repo.call(&["status", "--workflow"], true), "status")["workflow"]["saved_work"],
        "no_change"
    );
    repo.start("feature");
    repo.write("saved feature\n");
    repo.call(&["snapshot"], true);
    let pending = repo.call(&["status", "--workflow"], true);
    let workflow = &data(&pending, "status")["workflow"];
    assert_eq!(workflow["workspace_has_changes"], false);
    assert_eq!(workflow["saved_work"], "unintegrated");
    assert!(workflow["next_actions"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("integrate")));
    repo.call(&["line", "integrate"], true);
    let integrated = repo.call(&["status", "--workflow"], true);
    let workflow = &data(&integrated, "status")["workflow"];
    assert_eq!(workflow["saved_work"], "integrated");
    assert_eq!(workflow["materialized_matches_line"], true);
    repo.write("unsaved follow-up\n");
    assert_eq!(
        data(&repo.call(&["status", "--workflow"], true), "status")["workflow"]
            ["workspace_has_changes"],
        true
    );
    // Ordinary status remains lightweight and retains its previous schema.
    assert!(data(&repo.call(&["status"], true), "status")
        .get("workflow")
        .is_none());
}

#[test]
fn workflow_status_explains_divergence_without_changing_history() {
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
    let before = repo.call(&["op", "log"], true);
    let status = repo.call(&["status", "--workflow"], true);
    let workflow = &data(&status, "status")["workflow"];
    assert_eq!(workflow["saved_work"], "diverged");
    assert_eq!(workflow["workspace_has_changes"], false);
    assert_eq!(workflow["materialized_matches_line"], false);
    assert_eq!(
        workflow["next_actions"],
        serde_json::json!(["review_integration", "inspect_line"])
    );
    assert_eq!(before, repo.call(&["op", "log"], true));
}

#[test]
fn workflow_never_labels_restricted_history_as_ready() {
    let repo = Repo::new();
    repo.start("public");
    repo.write("private file\n");
    let saved = repo.call(
        &[
            "snapshot",
            "--domain",
            "admin",
            "--message",
            "private snapshot",
        ],
        true,
    );
    let id = data(&saved, "snapshot_created")["id"].as_str().unwrap();
    let report = repo.call(&["status", "--workflow"], true);
    assert!(!report.to_string().contains(id));
    assert!(!report.to_string().contains("private snapshot"));
    let workflow = &data(&report, "status")["workflow"];
    assert_eq!(workflow["saved_work"], "restricted");
    assert_eq!(
        workflow["materialized_matches_line"],
        serde_json::Value::Null
    );
    assert!(!workflow["next_actions"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("preview_push")));
}
