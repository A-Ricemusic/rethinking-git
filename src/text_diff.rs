//! Bounded per-file text previews without invoking external diff tools.
use super::*;
use std::io::Read;

const TEXT_LIMIT: u64 = 1_048_576;

fn quoted_path(prefix: &str, path: &str) -> String {
    let value = format!("{prefix}{path}");
    if value
        .bytes()
        .all(|b| b.is_ascii_graphic() && !matches!(b, b'"' | b'\\'))
    {
        return value;
    }
    let mut quoted = String::from("\"");
    for byte in value.bytes() {
        match byte {
            b'"' => quoted.push_str("\\\""),
            b'\\' => quoted.push_str("\\\\"),
            b'\n' => quoted.push_str("\\n"),
            b'\r' => quoted.push_str("\\r"),
            b'\t' => quoted.push_str("\\t"),
            32..=126 => quoted.push(char::from(byte)),
            _ => quoted.push_str(&format!("\\{byte:03o}")),
        }
    }
    quoted.push('"');
    quoted
}

fn mode(file: &FileEntry) -> &'static str {
    if file.symlink {
        "120000"
    } else if file.executable {
        "100755"
    } else {
        "100644"
    }
}

fn bytes(repo: &Repo, file: Option<&FileEntry>, working: bool) -> Result<Vec<u8>> {
    let Some(file) = file else {
        return Ok(Vec::new());
    };
    transaction::validate_working_key(&file.path)?;
    if file.symlink && file.executable {
        bail!("invalid symlink mode");
    }
    let mut bytes = Vec::new();
    if working {
        let path = transaction::working_path(&repo.root, &file.path)?;
        let flags = transaction::working_flags(&path)?;
        #[cfg(unix)]
        if flags != file.flags() {
            bail!("working file changed while preparing text diff");
        }
        if flags.symlink {
            bytes = transaction::read_link_bytes(&path)?;
        } else {
            fs::File::open(path)?
                .take(TEXT_LIMIT + 1)
                .read_to_end(&mut bytes)?;
        }
    } else {
        verify::open_blob(repo, &file.hash)?
            .take(TEXT_LIMIT + 1)
            .read_to_end(&mut bytes)?;
    }
    if bytes.len() as u64 != file.bytes || hash_bytes(&bytes) != file.hash {
        bail!("file content changed or failed verification while preparing text diff");
    }
    Ok(bytes)
}

fn text(bytes: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return None;
    }
    Some(text)
}

pub(super) fn print(
    repo: &Repo,
    actor: &Actor,
    previous: Vec<FileEntry>,
    current: Vec<FileEntry>,
    working: bool,
) -> Result<()> {
    let previous = diff_input(previous, actor);
    let current = diff_input(current, actor);
    let hidden = hidden_changed_paths(&previous.hidden_by_path, &current.hidden_by_path);
    let before = manifest_map(previous.visible);
    let after = manifest_map(current.visible);
    let paths: BTreeSet<_> = before.keys().chain(after.keys()).collect();
    for path in paths {
        let old = before.get(path);
        let new = after.get(path);
        if old == new {
            continue;
        }
        transaction::validate_working_key(path)?;
        let a = quoted_path("a/", path);
        let b = quoted_path("b/", path);
        let mut header = format!("diff --git {a} {b}\n");
        match (old, new) {
            (None, Some(new)) => header.push_str(&format!("new file mode {}\n", mode(new))),
            (Some(old), None) => header.push_str(&format!("deleted file mode {}\n", mode(old))),
            (Some(old), Some(new)) if mode(old) != mode(new) => {
                header.push_str(&format!("old mode {}\nnew mode {}\n", mode(old), mode(new)));
            }
            _ => {}
        }
        if let (Some(old), Some(new)) = (old, new) {
            if old.policy != new.policy {
                header.push_str(
                    "# access policy changed; policies are not encoded in a text patch\n",
                );
            }
        }
        if old
            .into_iter()
            .chain(new)
            .any(|file| file.bytes > TEXT_LIMIT)
        {
            println!("{header}# text preview omitted: file exceeds 1 MiB");
            continue;
        }
        let old_bytes = bytes(repo, old, false)?;
        let new_bytes = bytes(repo, new, working)?;
        print!("{header}");
        if old_bytes == new_bytes {
            continue;
        }
        let (Some(old_text), Some(new_text)) = (text(&old_bytes), text(&new_bytes)) else {
            println!("# binary or non-text content differs; text preview omitted");
            continue;
        };
        let diff = similar::TextDiff::configure()
            .timeout(std::time::Duration::from_millis(500))
            .diff_lines(old_text, new_text);
        print!(
            "{}",
            diff.unified_diff().context_radius(3).header(
                if old.is_some() { &a } else { "/dev/null" },
                if new.is_some() { &b } else { "/dev/null" }
            )
        );
    }
    if hidden > 0 {
        println!("# hidden: {hidden} restricted changed path(s)");
    }
    Ok(())
}
