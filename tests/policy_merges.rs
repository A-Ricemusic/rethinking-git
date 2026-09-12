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
        let repo = Self(std::env::temp_dir().join(format!("rgit-policy-{}", Uuid::new_v4())));
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
fn policy_only_restriction_survives_integration_and_is_visible_in_admin_diff() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("notes.txt"), "unchanged content").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate", "main", "--as", "admin"]);
    repo.change();
    repo.ok(&["access", "path", "notes.txt", "--domain", "admin"]);
    let diff = repo.ok(&["diff", "workspace", "--as", "admin"]);
    assert!(
        diff.contains("modified:\n  notes.txt"),
        "policy change omitted: {diff}"
    );
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate", "main", "--as", "admin"]);
    let public = repo.ok(&["line", "view", "main", "--as", "public"]);
    assert!(
        !public.contains("notes.txt"),
        "restricted path remained public: {public}"
    );
    assert!(repo
        .ok(&["line", "view", "main", "--as", "admin"])
        .contains("notes.txt"));
}

#[test]
fn concurrent_content_edit_and_restriction_conflict_instead_of_dropping_policy() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("notes.txt"), "base").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate", "main", "--as", "admin"]);
    let restricted = repo.change();
    repo.ok(&["access", "path", "notes.txt", "--domain", "admin"]);
    repo.ok(&["snapshot"]);
    repo.change();
    repo.ok(&["access", "path", "notes.txt", "--domain", "public"]);
    fs::write(repo.0.join("notes.txt"), "edited").unwrap();
    repo.ok(&["snapshot"]);
    repo.ok(&["line", "integrate", "main", "--as", "admin"]);
    fs::write(
        repo.0.join(".rgit/workspace.json"),
        format!(r#"{{"current_change":"{restricted}"}}"#),
    )
    .unwrap();
    let result = repo.run(&["line", "integrate", "main", "--as", "admin"]);
    assert!(
        !result.status.success(),
        "concurrent policy change was silently dropped"
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("integration blocked"));
}

#[test]
fn hidden_policy_changes_are_counted_without_disclosing_paths() {
    let repo = Repo::new();
    repo.change();
    fs::write(repo.0.join("restricted.txt"), "same content").unwrap();
    repo.ok(&["access", "path", "restricted.txt", "--domain", "team/one"]);
    repo.ok(&["snapshot"]);
    repo.ok(&["access", "path", "restricted.txt", "--domain", "team/two"]);
    let diff = repo.ok(&["diff", "workspace", "--as", "public"]);
    assert!(diff.contains("hidden: 1 restricted file(s)"), "{diff}");
    assert!(!diff.contains("restricted.txt"));
}
