use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use uuid::Uuid;

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let repo = Self(std::env::temp_dir().join(format!("rgit-import-{}", Uuid::new_v4())));
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
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const AUTHOR: &str = "Fixture Author <fixture@example.test>";
fn destination() -> PathBuf {
    std::env::temp_dir().join(format!("rgit-import-destination-{}", Uuid::new_v4()))
}
fn git(repo: &Repo, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .current_dir(&repo.0)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn commit(source: &Repo, message: &str) {
    git(source, &["add", "."]);
    git(
        source,
        &[
            "-c",
            "user.name=Original Author",
            "-c",
            "user.email=original@example.test",
            "commit",
            "-m",
            message,
        ],
    );
}

#[test]
fn imports_and_reexports_original_commit_ids_and_merge_topology() {
    for format in ["sha1", "sha256"] {
        let source = Repo(destination());
        fs::create_dir(&source.0).unwrap();
        git(
            &source,
            &["init", "-b", "main", &format!("--object-format={format}")],
        );
        fs::write(source.0.join("file.txt"), "base").unwrap();
        fs::create_dir(source.0.join("dir")).unwrap();
        fs::write(source.0.join("dir/雪.txt"), b"binary\0content").unwrap();
        fs::write(source.0.join("dir.txt"), "tree ordering").unwrap();
        commit(&source, "first original message");
        git(&source, &["checkout", "-b", "side"]);
        fs::write(source.0.join("side.txt"), "side").unwrap();
        commit(&source, "side original message");
        git(&source, &["checkout", "main"]);
        fs::write(source.0.join("main.txt"), "main").unwrap();
        commit(&source, "main original message");
        git(
            &source,
            &[
                "-c",
                "user.name=Merge Author",
                "-c",
                "user.email=merge@example.test",
                "merge",
                "--no-ff",
                "side",
                "-m",
                "original merge",
            ],
        );
        let original = git(&source, &["rev-parse", "HEAD"]);
        let native = Repo::new();
        native.ok(&[
            "git",
            "import",
            source.0.to_str().unwrap(),
            "--as",
            "admin",
            "--domain",
            "public",
        ]);
        native.ok(&["repo", "verify", "--as", "admin"]);
        native.ok(&["workspace", "restore", "--discard-changes", "--as", "admin"]);
        assert_eq!(
            fs::read(native.0.join("dir/雪.txt")).unwrap(),
            b"binary\0content"
        );
        let exported = Repo(destination());
        native.ok(&[
            "git",
            "export",
            exported.0.to_str().unwrap(),
            "--as",
            "admin",
        ]);
        assert_eq!(git(&exported, &["rev-parse", "HEAD"]), original);
        assert_eq!(
            git(&exported, &["rev-list", "--topo-order", "HEAD"]),
            git(&source, &["rev-list", "--topo-order", "HEAD"])
        );
        git(&exported, &["fsck", "--full", "--strict"]);
        fs::write(native.0.join("new.txt"), "native edit").unwrap();
        native.ok(&["snapshot", "-m", "native edit"]);
        native.ok(&["line", "integrate", "--as", "admin"]);
        let extended = Repo(destination());
        native.ok(&[
            "git",
            "export",
            extended.0.to_str().unwrap(),
            "--as",
            "admin",
            "--author",
            AUTHOR,
        ]);
        let original = String::from_utf8(original).unwrap();
        git(
            &extended,
            &["merge-base", "--is-ancestor", original.trim(), "HEAD"],
        );
    }
}

#[test]
fn import_refuses_nonempty_target_and_unsupported_entries_without_publishing() {
    let source = Repo(destination());
    fs::create_dir(&source.0).unwrap();
    git(&source, &["init", "-b", "main"]);
    fs::write(source.0.join("file.txt"), "source").unwrap();
    commit(&source, "source");
    let native = Repo::new();
    native.ok(&["git", "import", source.0.to_str().unwrap(), "--as", "admin"]);
    assert!(!native
        .run(&["git", "import", source.0.to_str().unwrap(), "--as", "admin"])
        .status
        .success());
    {
        let head = String::from_utf8(git(&source, &["rev-parse", "HEAD"])).unwrap();
        git(
            &source,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},submodule", head.trim()),
            ],
        );
        git(
            &source,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "-m",
                "submodule",
            ],
        );
        let empty = Repo::new();
        assert!(!empty
            .run(&["git", "import", source.0.to_str().unwrap(), "--as", "admin"])
            .status
            .success());
        assert_eq!(
            fs::read_dir(empty.0.join(".rgit/snapshots"))
                .unwrap()
                .count(),
            0
        );
        empty.ok(&["repo", "verify", "--as", "admin"]);
    }
}

#[cfg(unix)]
#[test]
fn round_trip_preserves_a_real_ssh_commit_signature() {
    let keys = Repo(destination());
    fs::create_dir(&keys.0).unwrap();
    let key = keys.0.join("signing-key");
    let generated = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f", key.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(generated.status.success());
    let allowed = keys.0.join("allowed-signers");
    fs::write(
        &allowed,
        format!(
            "signed@example.test {}",
            fs::read_to_string(key.with_extension("pub")).unwrap()
        ),
    )
    .unwrap();
    let source = Repo(destination());
    fs::create_dir(&source.0).unwrap();
    git(&source, &["init", "-b", "main"]);
    fs::write(source.0.join("file.txt"), "signed contents").unwrap();
    git(&source, &["add", "."]);
    git(
        &source,
        &[
            "-c",
            "user.name=Signed Fixture",
            "-c",
            "user.email=signed@example.test",
            "-c",
            "gpg.format=ssh",
            "-c",
            &format!("user.signingkey={}", key.display()),
            "commit",
            "-S",
            "-m",
            "signed original",
        ],
    );
    let native = Repo::new();
    native.ok(&[
        "git",
        "import",
        source.0.to_str().unwrap(),
        "--as",
        "admin",
        "--domain",
        "public",
    ]);
    let exported = Repo(destination());
    native.ok(&[
        "git",
        "export",
        exported.0.to_str().unwrap(),
        "--as",
        "admin",
    ]);
    assert_eq!(
        git(&exported, &["rev-parse", "HEAD"]),
        git(&source, &["rev-parse", "HEAD"])
    );
    git(
        &exported,
        &[
            "-c",
            &format!("gpg.ssh.allowedSignersFile={}", allowed.display()),
            "verify-commit",
            "HEAD",
        ],
    );
}

#[test]
fn importing_shared_history_into_another_line_reuses_native_ancestors() {
    let source = Repo(destination());
    fs::create_dir(&source.0).unwrap();
    git(&source, &["init", "-b", "main"]);
    fs::write(source.0.join("file.txt"), "base").unwrap();
    commit(&source, "base");
    let native = Repo::new();
    native.ok(&["git", "import", source.0.to_str().unwrap(), "--as", "admin"]);
    let first: serde_json::Value =
        serde_json::from_slice(&fs::read(native.0.join(".rgit/lines/main.json")).unwrap()).unwrap();
    fs::write(source.0.join("file.txt"), "next").unwrap();
    commit(&source, "next");
    native.ok(&[
        "git",
        "import",
        source.0.to_str().unwrap(),
        "--into",
        "remote/main",
        "--as",
        "admin",
    ]);
    assert_eq!(
        fs::read_dir(native.0.join(".rgit/snapshots"))
            .unwrap()
            .count(),
        2
    );
    let line: serde_json::Value =
        serde_json::from_slice(&fs::read(native.0.join(".rgit/lines/remote__main.json")).unwrap())
            .unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(
        &fs::read(native.0.join(format!(
            ".rgit/snapshots/{}.json",
            line["head_snapshot"].as_str().unwrap()
        )))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(snapshot["parent_snapshot"], first["head_snapshot"]);
    native.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn symlink_trees_round_trip_and_restore_with_platform_semantics() {
    for format in ["sha1", "sha256"] {
        let source = Repo(destination());
        fs::create_dir(&source.0).unwrap();
        git(
            &source,
            &["init", "-b", "main", &format!("--object-format={format}")],
        );
        // Create a Git symlink through the index, even on Windows without link privileges.
        fs::write(source.0.join("target-bytes"), "../missing-target").unwrap();
        let blob = String::from_utf8(git(&source, &["hash-object", "-w", "target-bytes"])).unwrap();
        git(
            &source,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("120000,{},link", blob.trim()),
            ],
        );
        git(
            &source,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "-m",
                "link",
            ],
        );
        let head = git(&source, &["rev-parse", "HEAD"]);
        let native = Repo::new();
        native.ok(&[
            "git",
            "import",
            source.0.to_str().unwrap(),
            "--as",
            "admin",
            "--domain",
            "public",
        ]);
        native.ok(&["workspace", "restore", "--discard-changes", "--as", "admin"]);
        #[cfg(unix)]
        assert_eq!(
            fs::read_link(native.0.join("link")).unwrap(),
            PathBuf::from("../missing-target")
        );
        #[cfg(not(unix))]
        assert_eq!(
            fs::read(native.0.join("link")).unwrap(),
            b"../missing-target"
        );
        // Capturing the materialization must retain the logical link type on Windows.
        native.ok(&["snapshot"]);
        let snapshots = fs::read_dir(native.0.join(".rgit/snapshots")).unwrap();
        for snapshot in snapshots {
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(snapshot.unwrap().path()).unwrap()).unwrap();
            assert_eq!(value["files"][0]["symlink"], true);
        }
        let exported = Repo(destination());
        native.ok(&[
            "git",
            "export",
            exported.0.to_str().unwrap(),
            "--as",
            "admin",
        ]);
        assert_eq!(git(&exported, &["rev-parse", "HEAD"]), head);
        native.ok(&["repo", "verify", "--as", "admin"]);
    }
}

#[test]
fn tracked_build_directory_names_survive_import_restore_and_capture() {
    let source = Repo(destination());
    fs::create_dir(&source.0).unwrap();
    git(&source, &["init", "-b", "main"]);
    for path in ["target/source.txt", "node_modules/vendored.txt"] {
        fs::create_dir_all(source.0.join(path).parent().unwrap()).unwrap();
        fs::write(source.0.join(path), path).unwrap();
    }
    commit(&source, "tracked user directories");
    let original = git(&source, &["rev-parse", "HEAD"]);
    let native = Repo::new();
    native.ok(&[
        "git",
        "import",
        source.0.to_str().unwrap(),
        "--as",
        "admin",
        "--domain",
        "public",
    ]);
    native.ok(&["workspace", "restore", "--discard-changes", "--as", "admin"]);
    for path in ["target/source.txt", "node_modules/vendored.txt"] {
        assert_eq!(fs::read(native.0.join(path)).unwrap(), path.as_bytes());
    }
    // Ignore future generated siblings while retaining already-tracked content.
    fs::write(native.0.join(".gitignore"), "target/\nnode_modules/\n").unwrap();
    fs::write(native.0.join("target/generated"), "ignored").unwrap();
    native.ok(&["snapshot"]);
    let workspace: serde_json::Value =
        serde_json::from_slice(&fs::read(native.0.join(".rgit/workspace.json")).unwrap()).unwrap();
    let change: serde_json::Value = serde_json::from_slice(
        &fs::read(native.0.join(format!(
            ".rgit/changes/{}.json",
            workspace["current_change"].as_str().unwrap()
        )))
        .unwrap(),
    )
    .unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(
        &fs::read(native.0.join(format!(
            ".rgit/snapshots/{}.json",
            change["current_snapshot"].as_str().unwrap()
        )))
        .unwrap(),
    )
    .unwrap();
    let paths: Vec<_> = snapshot["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"target/source.txt"));
    assert!(paths.contains(&"node_modules/vendored.txt"));
    assert!(!paths.contains(&"target/generated"));
    let exported = Repo(destination());
    native.ok(&[
        "git",
        "export",
        exported.0.to_str().unwrap(),
        "--as",
        "admin",
    ]);
    assert_eq!(git(&exported, &["rev-parse", "HEAD"]), original);
    native.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn restoring_another_snapshot_preserves_its_modes_when_captured_on_a_new_change() {
    let source = Repo(destination());
    fs::create_dir(&source.0).unwrap();
    git(&source, &["init", "-b", "main"]);
    fs::write(source.0.join("target-bytes"), "../target").unwrap();
    let blob = String::from_utf8(git(&source, &["hash-object", "-w", "target-bytes"])).unwrap();
    for mode in ["100755", "120000"] {
        git(
            &source,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("{mode},{},item", blob.trim()),
            ],
        );
        git(
            &source,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "-m",
                mode,
            ],
        );
    }
    let native = Repo::new();
    native.ok(&[
        "git",
        "import",
        source.0.to_str().unwrap(),
        "--as",
        "admin",
        "--domain",
        "public",
    ]);
    let read = |path: &str| -> serde_json::Value {
        serde_json::from_slice(&fs::read(native.0.join(".rgit").join(path)).unwrap()).unwrap()
    };
    let line = read("lines/main.json");
    let link = line["head_snapshot"].as_str().unwrap();
    let snapshot = read(&format!("snapshots/{link}.json"));
    let executable = snapshot["parent_snapshot"].as_str().unwrap();
    native.ok(&[
        "workspace",
        "restore",
        "--from",
        executable,
        "--discard-changes",
        "--as",
        "admin",
    ]);
    let current = read("workspace.json");
    assert!(
        !native
            .run(&[
                "workspace",
                "switch",
                current["current_change"].as_str().unwrap(),
                "--as",
                "admin"
            ])
            .status
            .success(),
        "mode-only unsaved change must prevent switching"
    );
    native.ok(&["change", "new", "restored regular mode"]);
    native.ok(&["snapshot"]);
    let workspace = read("workspace.json");
    let captured = read(&format!(
        "snapshots/{}.json",
        workspace["mode_snapshot"].as_str().unwrap()
    ));
    assert_eq!(captured["files"][0]["executable"], true);
    assert_ne!(captured["files"][0]["symlink"], true);
    native.ok(&[
        "workspace",
        "restore",
        "--from",
        link,
        "--discard-changes",
        "--as",
        "admin",
    ]);
    native.ok(&["snapshot"]);
    let workspace = read("workspace.json");
    let captured = read(&format!(
        "snapshots/{}.json",
        workspace["mode_snapshot"].as_str().unwrap()
    ));
    assert_eq!(captured["files"][0]["symlink"], true);
    assert_ne!(captured["files"][0]["executable"], true);
    native.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn reusing_a_regular_blob_as_a_link_still_validates_its_target() {
    let source = Repo(destination());
    fs::create_dir(&source.0).unwrap();
    git(&source, &["init", "-b", "main"]);
    fs::write(source.0.join("bytes"), b"invalid\0link").unwrap();
    let blob = String::from_utf8(git(&source, &["hash-object", "-w", "bytes"])).unwrap();
    for mode in ["100644", "120000"] {
        git(
            &source,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("{mode},{},item", blob.trim()),
            ],
        );
        git(
            &source,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "-m",
                mode,
            ],
        );
    }
    let native = Repo::new();
    let result = native.run(&["git", "import", source.0.to_str().unwrap(), "--as", "admin"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr)
        .contains("symlink target must be nonempty and contain no NUL"));
    assert_eq!(
        fs::read_dir(native.0.join(".rgit/snapshots"))
            .unwrap()
            .count(),
        0
    );
    native.ok(&["repo", "verify", "--as", "admin"]);
}
