use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_REPO: AtomicUsize = AtomicUsize::new(0);

struct TestRepo {
    root: PathBuf,
}

impl TestRepo {
    fn new(name: &str) -> Self {
        let sequence = NEXT_REPO.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("rgit-cli-{name}-{}-{sequence}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale test repository");
        }
        fs::create_dir_all(&root).expect("create test repository");
        Self { root }
    }

    fn run(&self, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_rgit"))
            .args(args)
            .current_dir(&self.root)
            .output()
            .unwrap_or_else(|error| panic!("failed to run rgit {:?}: {error}", args));
        successful_stdout(args, output)
    }

    fn run_refused(&self, args: &[&str], expected_error: &str) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_rgit"))
            .args(args)
            .current_dir(&self.root)
            .output()
            .unwrap_or_else(|error| panic!("failed to run rgit {:?}: {error}", args));
        refused_stdout(args, output, expected_error)
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.root.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture parent directory");
        }
        fs::write(path, contents).expect("write fixture file");
    }

    fn use_change(&self, change_id: &str) {
        let document = format!("{{\n  \"current_change\": \"{change_id}\"\n}}\n");
        fs::write(self.root.join(".rgit/workspace.json"), document)
            .expect("select test workspace change");
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "preserving failed test repository at {}",
                self.root.display()
            );
        } else {
            fs::remove_dir_all(&self.root).expect("remove test repository");
        }
    }
}

fn successful_stdout(args: &[&str], output: Output) -> String {
    if !output.status.success() {
        panic!(
            "rgit {:?} failed with {}\nstdout:\n{}\nstderr:\n{}",
            args,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        output.stderr.is_empty(),
        "rgit {:?} unexpectedly wrote to stderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("rgit stdout is UTF-8")
}

fn refused_stdout(args: &[&str], output: Output, expected_error: &str) -> String {
    assert_eq!(
        output.status.code(),
        Some(1),
        "rgit {:?} should report a refused operation with exit code 1",
        args
    );
    assert_eq!(
        String::from_utf8(output.stderr)
            .expect("rgit stderr is UTF-8")
            .replace("\r\n", "\n"),
        format!("Error: {expected_error}\n"),
        "rgit {:?} exposed an unexpected refusal error",
        args
    );
    String::from_utf8(output.stdout).expect("rgit stdout is UTF-8")
}

fn id_after(output: &str, prefix: &str) -> String {
    output
        .split_whitespace()
        .find(|word| word.starts_with(prefix))
        .unwrap_or_else(|| panic!("missing {prefix} identifier in {output:?}"))
        .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .to_string()
}

fn normalize_ids(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let remainder = &value[index..];
        let replacement = if remainder.starts_with("snap_") {
            Some(("snap_", "<SNAP>"))
        } else if remainder.starts_with("chg_") {
            Some(("chg_", "<CHANGE>"))
        } else if remainder.starts_with("conf_") {
            Some(("conf_", "<CONFLICT>"))
        } else if remainder.starts_with("op_") {
            Some(("op_", "<OP>"))
        } else {
            None
        };

        if let Some((prefix, placeholder)) = replacement {
            let mut end = index + prefix.len();
            while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                end += 1;
            }
            if end > index + prefix.len() {
                normalized.push_str(placeholder);
                index = end;
                continue;
            }
        }

        let character = remainder.chars().next().expect("valid character boundary");
        normalized.push(character);
        index += character.len_utf8();
    }
    normalized
}

fn section(transcript: &mut String, command: &str, output: &str) {
    transcript.push_str("$ rgit ");
    transcript.push_str(command);
    transcript.push('\n');
    transcript.push_str(output);
}

fn sorted_lines(output: &str) -> String {
    let mut lines = output.lines().collect::<Vec<_>>();
    lines.sort_unstable_by_key(|line| line.split_once(' ').map_or(*line, |(_, rest)| rest));
    let mut result = lines.join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    result
}

fn assert_golden(actual: &str, expected: &str) {
    assert_eq!(
        normalize_ids(actual),
        expected.replace("\r\n", "\n"),
        "golden CLI transcript changed"
    );
}

#[test]
fn public_cli_flow_matches_golden_transcript() {
    let repo = TestRepo::new("public");
    repo.write("README.md", "hello\n");
    let mut transcript = String::new();

    section(&mut transcript, "init", &repo.run(&["init"]));
    section(&mut transcript, "line list", &repo.run(&["line", "list"]));
    section(&mut transcript, "status", &repo.run(&["status"]));

    let created = repo.run(&["change", "new", "document-cli"]);
    let change = id_after(&created, "chg_");
    section(&mut transcript, "change new document-cli", &created);
    section(&mut transcript, "status", &repo.run(&["status"]));
    section(
        &mut transcript,
        "diff workspace",
        &repo.run(&["diff", "workspace"]),
    );

    let created = repo.run(&["snapshot", "--message", "document public CLI"]);
    let snapshot = id_after(&created, "snap_");
    section(
        &mut transcript,
        "snapshot --message 'document public CLI'",
        &created,
    );
    section(
        &mut transcript,
        "workspace info",
        &repo.run(&["workspace", "info"]),
    );
    section(
        &mut transcript,
        "change list",
        &repo.run(&["change", "list"]),
    );
    section(
        &mut transcript,
        "change show <CHANGE>",
        &repo.run(&["change", "show", &change]),
    );
    section(
        &mut transcript,
        "snapshot-info list",
        &repo.run(&["snapshot-info", "list"]),
    );
    section(
        &mut transcript,
        "snapshot-info show <SNAP>",
        &repo.run(&["snapshot-info", "show", &snapshot]),
    );
    section(
        &mut transcript,
        "merge preview --into main",
        &repo.run(&["merge", "preview", "--into", "main"]),
    );
    section(
        &mut transcript,
        "line integrate main",
        &repo.run(&["line", "integrate", "main"]),
    );
    section(
        &mut transcript,
        "line view main",
        &repo.run(&["line", "view", "main"]),
    );
    section(
        &mut transcript,
        "line history main",
        &repo.run(&["line", "history", "main"]),
    );
    section(
        &mut transcript,
        "diff line main",
        &repo.run(&["diff", "line", "main"]),
    );
    section(
        &mut transcript,
        "op log (sorted for same-second operations)",
        &sorted_lines(&repo.run(&["op", "log"])),
    );

    assert_golden(&transcript, include_str!("golden/public_flow.txt"));
}

#[test]
fn new_change_uses_populated_line_as_permissioned_workspace_base() {
    let repo = TestRepo::new("populated-line-base");
    repo.write("README.md", "base\n");
    repo.write(".env", "SECRET=base\n");
    repo.run(&["init"]);
    repo.run(&["access", "path", ".env", "--domain", "admin"]);
    repo.run(&["change", "new", "base"]);
    repo.run(&["snapshot", "--message", "establish base"]);
    repo.run(&["line", "integrate", "main", "--as", "admin"]);

    let mut transcript = String::new();
    section(
        &mut transcript,
        "change new follow-up",
        &repo.run(&["change", "new", "follow-up"]),
    );
    section(&mut transcript, "status", &repo.run(&["status"]));
    section(
        &mut transcript,
        "diff workspace",
        &repo.run(&["diff", "workspace"]),
    );

    repo.write("README.md", "updated\n");
    repo.write(".env", "SECRET=rotated\n");
    section(
        &mut transcript,
        "status after edits",
        &repo.run(&["status"]),
    );
    section(
        &mut transcript,
        "diff workspace after edits",
        &repo.run(&["diff", "workspace"]),
    );

    assert!(!transcript.contains(".env"));
    assert!(!transcript.contains("SECRET"));
    assert_golden(&transcript, include_str!("golden/populated_line_base.txt"));
}

#[test]
fn permissioned_security_flow_matches_golden_transcript() {
    let repo = TestRepo::new("security");
    let mut transcript = String::new();
    repo.run(&["init"]);
    section(
        &mut transcript,
        "actor set alice --domain public",
        &repo.run(&["actor", "set", "alice", "--domain", "public"]),
    );
    section(
        &mut transcript,
        "actor set bob --domain public --domain team/security",
        &repo.run(&[
            "actor",
            "set",
            "bob",
            "--domain",
            "public",
            "--domain",
            "team/security",
        ]),
    );
    section(
        &mut transcript,
        "access path .env --domain admin",
        &repo.run(&["access", "path", ".env", "--domain", "admin"]),
    );
    section(
        &mut transcript,
        "access path security --domain team/security",
        &repo.run(&["access", "path", "security", "--domain", "team/security"]),
    );
    section(&mut transcript, "actor list", &repo.run(&["actor", "list"]));
    section(
        &mut transcript,
        "access list",
        &repo.run(&["access", "list"]),
    );

    let created = repo.run(&[
        "change",
        "new",
        "fix-token-replay",
        "--domain",
        "team/security",
    ]);
    let change = id_after(&created, "chg_");
    repo.write("src/auth.txt", "patched auth\n");
    repo.write(".env", "SECRET=value\n");
    repo.write("security/repro.test", "exploit repro\n");
    let created = repo.run(&["snapshot", "--message", "fix token replay"]);
    let first_snapshot = id_after(&created, "snap_");
    repo.write("src/auth.txt", "patched auth v2\n");
    repo.write(".env", "SECRET=rotated\n");
    let created = repo.run(&["snapshot", "--message", "harden token replay fix"]);
    let snapshot = id_after(&created, "snap_");

    section(
        &mut transcript,
        "change list --as alice",
        &repo.run(&["change", "list", "--as", "alice"]),
    );
    section(
        &mut transcript,
        "change show <CHANGE> --as alice",
        &repo.run(&["change", "show", &change, "--as", "alice"]),
    );
    section(
        &mut transcript,
        "change list --as bob",
        &repo.run(&["change", "list", "--as", "bob"]),
    );
    section(
        &mut transcript,
        "snapshot-info list --as bob",
        &repo.run(&["snapshot-info", "list", "--as", "bob"]),
    );
    section(
        &mut transcript,
        "snapshot-info show <SNAP> --as bob",
        &repo.run(&["snapshot-info", "show", &snapshot, "--as", "bob"]),
    );
    section(
        &mut transcript,
        "diff snapshot <SNAP> <SNAP> --as bob",
        &repo.run(&[
            "diff",
            "snapshot",
            &first_snapshot,
            &snapshot,
            "--as",
            "bob",
        ]),
    );
    section(
        &mut transcript,
        "merge preview --into main --as alice",
        &repo.run_refused(
            &["merge", "preview", "--into", "main", "--as", "alice"],
            "operation unavailable",
        ),
    );
    section(
        &mut transcript,
        "merge preview --into main --as bob",
        &repo.run_refused(
            &["merge", "preview", "--into", "main", "--as", "bob"],
            "operation unavailable",
        ),
    );
    section(
        &mut transcript,
        "merge preview --into main --as admin",
        &repo.run(&["merge", "preview", "--into", "main", "--as", "admin"]),
    );
    section(
        &mut transcript,
        "line integrate main --as bob",
        &repo.run_refused(
            &["line", "integrate", "main", "--as", "bob"],
            "operation unavailable",
        ),
    );
    section(
        &mut transcript,
        "line integrate main --as admin",
        &repo.run(&["line", "integrate", "main", "--as", "admin"]),
    );
    section(
        &mut transcript,
        "line view main --as alice",
        &repo.run(&["line", "view", "main", "--as", "alice"]),
    );
    section(
        &mut transcript,
        "line view main --as bob",
        &repo.run(&["line", "view", "main", "--as", "bob"]),
    );
    section(
        &mut transcript,
        "line view main --as admin",
        &repo.run(&["line", "view", "main", "--as", "admin"]),
    );
    section(
        &mut transcript,
        "diff workspace --as alice",
        &repo.run(&["diff", "workspace", "--as", "alice"]),
    );
    section(
        &mut transcript,
        "line history main --as alice",
        &repo.run(&["line", "history", "main", "--as", "alice"]),
    );
    section(
        &mut transcript,
        "line history main --as bob",
        &repo.run(&["line", "history", "main", "--as", "bob"]),
    );
    section(
        &mut transcript,
        "line history main --as admin",
        &repo.run(&["line", "history", "main", "--as", "admin"]),
    );
    section(
        &mut transcript,
        "op log --as alice (sorted)",
        &sorted_lines(&repo.run(&["op", "log", "--as", "alice"])),
    );
    section(
        &mut transcript,
        "op log --as bob (sorted)",
        &sorted_lines(&repo.run(&["op", "log", "--as", "bob"])),
    );

    let normalized = normalize_ids(&transcript);
    assert!(!normalized.contains("SECRET=value"));
    assert!(!normalized.contains("exploit repro"));
    assert_golden(&transcript, include_str!("golden/security_flow.txt"));
}

#[test]
fn hidden_unsnapshotted_change_has_generic_merge_refusals() {
    let repo = TestRepo::new("hidden-unsnapshotted-change");
    repo.run(&["init"]);
    repo.run(&["actor", "set", "outsider", "--domain", "public"]);
    repo.run(&[
        "change",
        "new",
        "embargoed-security-fix",
        "--domain",
        "team/security",
    ]);

    let mut transcript = String::new();
    let preview = repo.run_refused(
        &["merge", "preview", "--into", "main", "--as", "outsider"],
        "operation unavailable",
    );
    assert!(preview.is_empty(), "merge preview leaked sensitive stdout");
    section(
        &mut transcript,
        "merge preview --into main --as outsider",
        &preview,
    );

    let integrate = repo.run_refused(
        &["line", "integrate", "main", "--as", "outsider"],
        "operation unavailable",
    );
    assert!(
        integrate.is_empty(),
        "line integrate leaked sensitive stdout"
    );
    section(
        &mut transcript,
        "line integrate main --as outsider",
        &integrate,
    );

    assert_golden(
        &transcript,
        include_str!("golden/hidden_unsnapshotted_change.txt"),
    );
}

#[test]
fn divergent_changes_create_permission_aware_conflict() {
    let repo = TestRepo::new("conflict");
    repo.run(&["init"]);
    repo.write("app.txt", "base\n");

    repo.run(&["change", "new", "base"]);
    let base_output = repo.run(&["snapshot", "--message", "base"]);
    let base_snapshot = id_after(&base_output, "snap_");
    repo.run(&["line", "integrate", "main"]);

    let incoming_output = repo.run(&["change", "new", "incoming"]);
    let incoming_change = id_after(&incoming_output, "chg_");
    repo.write("app.txt", "incoming\n");
    let incoming_output = repo.run(&["snapshot", "--message", "incoming edit"]);
    let incoming_snapshot = id_after(&incoming_output, "snap_");

    repo.run(&["change", "new", "line-update"]);
    repo.write("app.txt", "line\n");
    repo.run(&["snapshot", "--message", "line edit"]);
    repo.run(&["line", "integrate", "main"]);
    repo.use_change(&incoming_change);

    let mut transcript = String::new();
    section(
        &mut transcript,
        "diff snapshot <SNAP> <SNAP>",
        &repo.run(&["diff", "snapshot", &base_snapshot, &incoming_snapshot]),
    );
    section(
        &mut transcript,
        "merge preview <CHANGE> --into main",
        &repo.run(&["merge", "preview", &incoming_change, "--into", "main"]),
    );
    let integrate = repo.run_refused(
        &["line", "integrate", "main"],
        "integration blocked by conflicts",
    );
    let conflict = id_after(&integrate, "conf_");
    section(&mut transcript, "line integrate main", &integrate);
    section(
        &mut transcript,
        "conflict list",
        &repo.run(&["conflict", "list"]),
    );
    section(
        &mut transcript,
        "conflict show <CONFLICT>",
        &repo.run(&["conflict", "show", &conflict]),
    );

    assert_golden(&transcript, include_str!("golden/conflict_flow.txt"));
}

#[test]
fn commands_fail_cleanly_outside_a_repository() {
    let repo = TestRepo::new("not-a-repo");
    let output = Command::new(env!("CARGO_BIN_EXE_rgit"))
        .args(["status"])
        .current_dir(&repo.root)
        .output()
        .expect("run rgit status");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr)
            .expect("stderr is UTF-8")
            .replace("\r\n", "\n"),
        "Error: not inside an rgit repository; run `rgit init` first\n"
    );
}

#[test]
fn golden_files_use_platform_independent_newlines() {
    for path in [
        Path::new("tests/golden/public_flow.txt"),
        Path::new("tests/golden/populated_line_base.txt"),
        Path::new("tests/golden/security_flow.txt"),
        Path::new("tests/golden/conflict_flow.txt"),
    ] {
        let contents = fs::read(path).unwrap_or_else(|error| {
            panic!("failed to read golden file {}: {error}", path.display())
        });
        assert!(
            !contents.windows(2).any(|window| window == b"\r\n"),
            "{} contains CRLF newlines",
            path.display()
        );
    }
}
