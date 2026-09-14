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

#[test]
fn denied_and_missing_direct_reads_fail_without_disclosing_existence() {
    let repo = Repo::new();
    repo.ok(&["change", "new", "private", "--domain", "admin"]);
    let workspace: Value =
        serde_json::from_slice(&fs::read(repo.0.join(".rgit/workspace.json")).unwrap()).unwrap();
    let change = workspace["current_change"].as_str().unwrap();
    repo.ok(&["snapshot", "--message", "private-message"]);
    let snapshot = snapshot_id(&repo);
    let line_path = repo.0.join(".rgit/lines/main.json");
    let mut line: Value = serde_json::from_slice(&fs::read(&line_path).unwrap()).unwrap();
    line["policy"]["domains"] = serde_json::json!(["admin"]);
    fs::write(&line_path, serde_json::to_vec(&line).unwrap()).unwrap();
    let operations = fs::read_dir(repo.0.join(".rgit/operations"))
        .unwrap()
        .count();
    for args in [
        vec!["change", "show", change],
        vec!["change", "show", "chg_00000000000000000000000000000000"],
        vec!["snapshot-info", "show", &snapshot],
        vec![
            "snapshot-info",
            "show",
            "snap_00000000000000000000000000000000",
        ],
        vec!["status"],
        vec!["diff", "workspace"],
        vec!["diff", "snapshot", &snapshot, &snapshot],
        vec![
            "diff",
            "snapshot",
            &snapshot,
            "snap_00000000000000000000000000000000",
        ],
        vec!["line", "view", "main"],
        vec!["line", "view", "missing"],
        vec!["line", "history", "main"],
        vec!["line", "history", "missing"],
        vec!["diff", "line", "main"],
        vec!["diff", "line", "missing"],
        vec!["conflict", "show", "conf_00000000000000000000000000000000"],
    ] {
        let output = repo.run(&args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n"),
            "Error: operation unavailable\n",
            "{args:?}"
        );
    }
    assert_eq!(
        fs::read_dir(repo.0.join(".rgit/operations"))
            .unwrap()
            .count(),
        operations
    );
    assert!(repo
        .run(&["change", "show", change, "--as", "admin"])
        .status
        .success());
}

#[test]
fn corrupt_conflict_read_is_an_error_instead_of_a_successful_empty_result() {
    let repo = Repo::new();
    let id = "conf_00000000000000000000000000000000";
    fs::write(
        repo.0.join(format!(".rgit/conflicts/{id}.json")),
        b"not json",
    )
    .unwrap();
    let result = repo.run(&["conflict", "show", id]);
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("operation unavailable"));
}

#[test]
fn json_status_has_versioned_empty_and_permission_filtered_states() {
    let repo = Repo::new();
    let empty: Value = serde_json::from_str(&repo.ok(&["status", "--json"])).unwrap();
    assert_eq!(
        empty,
        serde_json::json!({
            "schema_version": 1, "command": "status", "actor": "public",
            "change": null, "base_snapshot": {"state": "absent"}, "materialized_snapshot": {"state": "absent"}, "changes": null,
        })
    );
    let change = repo.change();
    repo.ok(&["access", "path", "secret.txt", "--domain", "admin"]);
    fs::write(repo.0.join("visible.txt"), "before").unwrap();
    fs::write(repo.0.join("deleted.txt"), "before").unwrap();
    fs::write(repo.0.join("secret.txt"), "before").unwrap();
    repo.ok(&[
        "snapshot",
        "--domain",
        "admin",
        "--message",
        "private-message",
    ]);
    let hidden_snapshot = snapshot_id(&repo);
    fs::write(repo.0.join("visible.txt"), "after").unwrap();
    fs::write(repo.0.join("secret.txt"), "after").unwrap();
    fs::write(repo.0.join("added with spaces.txt"), "new").unwrap();
    fs::remove_file(repo.0.join("deleted.txt")).unwrap();
    let operations = fs::read_dir(repo.0.join(".rgit/operations"))
        .unwrap()
        .count();
    let output = repo.ok(&["status", "--json"]);
    assert_eq!(output.lines().count(), 1);
    for restricted in [&hidden_snapshot, "private-message", "secret.txt"] {
        assert!(!output.contains(restricted));
    }
    let report: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(
        report,
        serde_json::json!({
            "schema_version": 1, "command": "status", "actor": "public",
            "change": {"id": change, "name": "test"},
            "base_snapshot": {"state": "restricted"},
            "materialized_snapshot": {"state": "restricted"},
            "changes": {"added": ["added with spaces.txt"], "modified": ["visible.txt"], "deleted": ["deleted.txt"], "hidden_count": 1},
        })
    );
    let admin: Value =
        serde_json::from_str(&repo.ok(&["status", "--json", "--as", "admin"])).unwrap();
    assert_eq!(
        admin["base_snapshot"],
        serde_json::json!({"state": "visible", "id": hidden_snapshot})
    );
    assert_eq!(admin["changes"]["hidden_count"], 0);
    assert_eq!(
        fs::read_dir(repo.0.join(".rgit/operations"))
            .unwrap()
            .count(),
        operations
    );
}

#[test]
fn json_status_reports_inherited_base_and_refuses_restricted_changes() {
    let repo = Repo::new();
    repo.change();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate", "--as", "admin"]);
    let line: Value =
        serde_json::from_slice(&fs::read(repo.0.join(".rgit/lines/main.json")).unwrap()).unwrap();
    repo.change();
    let report: Value = serde_json::from_str(&repo.ok(&["status", "--json"])).unwrap();
    assert_eq!(
        report["base_snapshot"],
        serde_json::json!({"state": "visible", "id": line["head_snapshot"]})
    );
    assert_eq!(
        report["changes"],
        serde_json::json!({"added": [], "modified": [], "deleted": [], "hidden_count": 0})
    );
    repo.ok(&["change", "new", "private", "--domain", "admin"]);
    let output = repo.run(&["status", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("operation unavailable"));
}

#[cfg(unix)]
#[test]
fn json_status_preserves_newlines_quotes_and_backslashes_in_paths() {
    let repo = Repo::new();
    repo.change();
    let name = "line\nbreak\"back\\slash";
    fs::write(repo.0.join(name), "contents").unwrap();
    let output = repo.ok(&["status", "--json"]);
    assert_eq!(output.lines().count(), 1);
    let report: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(report["changes"]["added"], serde_json::json!([name]));
}
