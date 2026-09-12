use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use uuid::Uuid;

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let repo = Self(std::env::temp_dir().join(format!("rgit-boundaries-{}", Uuid::new_v4())));
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

#[test]
fn unsupported_repository_formats_are_rejected_without_mutation() {
    for version in [0, 1, 3, 999] {
        let repo = Repo::new();
        let path = repo.0.join(".rgit/repo.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        config["format_version"] = json!(version);
        fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
        let workspace = fs::read(repo.0.join(".rgit/workspace.json")).unwrap();
        let result = repo.run(&["change", "new", "must-not-exist"]);
        assert!(!result.status.success(), "accepted format {version}");
        assert!(String::from_utf8_lossy(&result.stderr).contains("unsupported repository format"));
        assert_eq!(
            fs::read(repo.0.join(".rgit/workspace.json")).unwrap(),
            workspace
        );
        assert_eq!(
            fs::read_dir(repo.0.join(".rgit/changes")).unwrap().count(),
            0
        );
    }
}

#[test]
fn missing_config_is_not_treated_as_an_initialized_repository() {
    let repo = Repo::new();
    fs::remove_file(repo.0.join(".rgit/repo.json")).unwrap();
    assert!(!repo.run(&["status"]).status.success());
}

#[test]
fn object_ids_cannot_be_used_as_paths_or_as_other_object_kinds() {
    let repo = Repo::new();
    let id = repo.change();
    let traversal = format!("../changes/{id}");
    let absolute = repo
        .0
        .join(".rgit/changes")
        .join(&id)
        .to_string_lossy()
        .into_owned();
    for candidate in [
        traversal.as_str(),
        absolute.as_str(),
        "..\\changes\\anything",
        "snap_000000000000",
    ] {
        let output = repo.run(&["change", "show", candidate]);
        assert!(!output.status.success(), "accepted {candidate}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("invalid object identifier"));
    }
    assert!(repo.run(&["change", "show", &id]).status.success());
}

#[test]
fn actor_aliases_cannot_overwrite_or_impersonate_an_existing_actor() {
    let repo = Repo::new();
    repo.ok(&["actor", "set", "team/alice", "--domain", "public"]);
    let before = fs::read(repo.0.join(".rgit/actors/team__alice.json")).unwrap();
    assert!(!repo
        .run(&["actor", "set", "team__alice", "--domain", "admin"])
        .status
        .success());
    assert_eq!(
        fs::read(repo.0.join(".rgit/actors/team__alice.json")).unwrap(),
        before
    );
    assert!(!repo
        .run(&["status", "--as", "team__alice"])
        .status
        .success());
    assert!(repo.run(&["status", "--as", "team/alice"]).status.success());
}

#[test]
fn mismatched_embedded_identity_is_rejected() {
    let repo = Repo::new();
    let id = repo.change();
    let path = repo.0.join(format!(".rgit/changes/{id}.json"));
    let mut change: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    change["id"] = json!("../../outside");
    fs::write(&path, serde_json::to_vec(&change).unwrap()).unwrap();
    assert!(!repo.run(&["snapshot"]).status.success());
    assert_eq!(
        fs::read_dir(repo.0.join(".rgit/snapshots"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn actor_names_reject_platform_path_syntax() {
    let repo = Repo::new();
    for name in [
        "", "..", "../admin", "a\\b", "C:admin", "a\nb", "con", "a/../b", "a//b", "a.",
    ] {
        assert!(
            !repo.run(&["actor", "set", name]).status.success(),
            "accepted {name:?}"
        );
    }
}

#[test]
fn valid_unicode_actor_names_remain_readable_and_updatable() {
    let repo = Repo::new();
    repo.ok(&["actor", "set", "équipe/alice", "--domain", "public"]);
    repo.ok(&["actor", "set", "équipe/alice", "--domain", "admin"]);
    assert!(repo
        .ok(&["actor", "list"])
        .contains("équipe/alice domains:admin"));
    repo.ok(&["status", "--as", "équipe/alice"]);
}

#[test]
fn new_identifiers_keep_full_uuid_entropy_and_legacy_changes_remain_readable() {
    let repo = Repo::new();
    let full_id = repo.change();
    assert_eq!(full_id.strip_prefix("chg_").unwrap().len(), 32);
    let legacy_id = &full_id[..16];
    let full_path = repo.0.join(format!(".rgit/changes/{full_id}.json"));
    let legacy_path = repo.0.join(format!(".rgit/changes/{legacy_id}.json"));
    let mut change: Value = serde_json::from_slice(&fs::read(&full_path).unwrap()).unwrap();
    change["id"] = json!(legacy_id);
    fs::write(&legacy_path, serde_json::to_vec(&change).unwrap()).unwrap();
    fs::remove_file(full_path).unwrap();
    fs::write(
        repo.0.join(".rgit/workspace.json"),
        serde_json::to_vec(&json!({"current_change": legacy_id})).unwrap(),
    )
    .unwrap();
    repo.ok(&["change", "show", legacy_id]);
    repo.ok(&["snapshot"]);
    let updated: Value = serde_json::from_slice(&fs::read(legacy_path).unwrap()).unwrap();
    let snapshot = updated["current_snapshot"].as_str().unwrap();
    assert_eq!(snapshot.strip_prefix("snap_").unwrap().len(), 32);
    repo.ok(&["snapshot-info", "show", snapshot]);
}
