//! Conservative three-way text merging. Only disjoint base-line edits (or
//! identical edits) combine; ambiguous boundaries remain explicit conflicts.
use super::*;
use std::{io::Read, time::Duration};

const FILE_LIMIT: u64 = 1_048_576;
const TOTAL_LIMIT: usize = 64 * 1_048_576;
pub(super) type Generated = BTreeMap<String, Vec<u8>>;

#[derive(Debug, PartialEq, Eq)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

fn edits(base: &str, side: &str) -> Vec<Edit> {
    let diff = similar::TextDiff::configure()
        .timeout(Duration::from_millis(500))
        .diff_lines(base, side);
    diff.ops()
        .iter()
        .filter(|op| op.tag() != similar::DiffTag::Equal)
        .map(|op| {
            let range = op.old_range();
            Edit {
                start: range.start,
                end: range.end,
                replacement: diff.new_slices()[op.new_range()].concat(),
            }
        })
        .collect()
}

fn merge(base: &str, line: &str, incoming: &str) -> Option<Vec<u8>> {
    let mut changes = edits(base, line);
    changes.extend(edits(base, incoming));
    changes.sort_by(|a, b| (a.start, a.end, &a.replacement).cmp(&(b.start, b.end, &b.replacement)));
    changes.dedup();
    for pair in changes.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if a.end > b.start || (a.end == b.start && (a.start == a.end || b.start == b.end)) {
            return None;
        }
    }
    let lines = similar::DiffableStr::tokenize_lines(base);
    let mut result = Vec::new();
    let mut cursor = 0;
    for change in changes {
        for line in &lines[cursor..change.start] {
            result.extend_from_slice(line.as_bytes());
        }
        result.extend_from_slice(change.replacement.as_bytes());
        cursor = change.end;
    }
    for line in &lines[cursor..] {
        result.extend_from_slice(line.as_bytes());
    }
    (result.len() <= FILE_LIMIT as usize).then_some(result)
}

fn read_text(repo: &Repo, file: &FileEntry) -> Result<Option<String>> {
    let mut bytes = Vec::new();
    verify::open_blob(repo, &file.hash)?
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != file.bytes || hash_bytes(&bytes) != file.hash {
        bail!("text merge source blob failed verification");
    }
    Ok(String::from_utf8(bytes).ok().filter(|text| {
        !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    }))
}

pub(super) fn apply(
    repo: &Repo,
    plan: &mut MergePlan,
    base: &Option<Snapshot>,
    line: &Option<Snapshot>,
    incoming: &Snapshot,
) -> Result<Generated> {
    let sources = [base.as_ref(), line.as_ref(), Some(incoming)];
    let maps: Vec<BTreeMap<&str, &FileEntry>> = sources
        .iter()
        .map(|snapshot| {
            snapshot
                .iter()
                .flat_map(|s| &s.files)
                .map(|file| (file.path.as_str(), file))
                .collect()
        })
        .collect();
    let mut generated = Generated::new();
    let mut total = 0;
    let mut unresolved = Vec::new();
    for pending in std::mem::take(&mut plan.conflicts) {
        let sides: Vec<_> = maps
            .iter()
            .filter_map(|map| map.get(pending.path.as_str()).copied())
            .collect();
        let eligible = pending.kind == ConflictKind::BothModified
            && sides.len() == 3
            && sides.iter().all(|file| {
                !file.symlink
                    && file.bytes <= FILE_LIMIT
                    && file.flags() == sides[0].flags()
                    && file.policy == sides[0].policy
            })
            && total + FILE_LIMIT as usize <= TOTAL_LIMIT;
        let merged = if eligible {
            let texts = sides
                .iter()
                .map(|file| read_text(repo, file))
                .collect::<Result<Vec<_>>>()?;
            match (&texts[0], &texts[1], &texts[2]) {
                (Some(base), Some(line), Some(incoming)) => merge(base, line, incoming),
                _ => None,
            }
        } else {
            None
        };
        if let Some(bytes) = merged {
            let mut file = sides[0].clone();
            file.hash = hash_bytes(&bytes);
            file.bytes = bytes.len() as u64;
            total += bytes.len();
            generated.insert(file.hash.clone(), bytes);
            plan.merged_files.push(file);
        } else {
            unresolved.push(pending);
        }
    }
    plan.conflicts = unresolved;
    plan.merged_files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(generated)
}

pub(super) fn verify(repo: &Repo, plan: &MergePlan, generated: &Generated) -> Result<()> {
    verify::verify_manifest_paths(&plan.merged_files)?;
    for file in &plan.merged_files {
        if let Some(bytes) = generated.get(&file.hash) {
            if file.symlink || bytes.len() as u64 != file.bytes || hash_bytes(bytes) != file.hash {
                bail!("generated merge content failed verification");
            }
        } else {
            verify::verify_file(repo, file)?;
        }
    }
    Ok(())
}

pub(super) fn publish(repo: &Repo, generated: &Generated) -> Result<()> {
    for (hash, bytes) in generated {
        let path = repo.path(&["blobs", hash]);
        if path.try_exists()? {
            let mut stored = Vec::new();
            verify::open_blob(repo, hash)?
                .take(FILE_LIMIT + 1)
                .read_to_end(&mut stored)?;
            if stored != *bytes {
                bail!("existing merge blob failed verification");
            }
        } else {
            transaction::publish_file(&path, bytes)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_independent_edits_and_preserves_line_endings_and_final_bytes() {
        for separator in ["\n", "\r\n", "\r"] {
            let base = ["one", "two", "three"].join(separator);
            let line = ["ONE", "two", "three"].join(separator);
            let incoming = ["one", "two", "THREE"].join(separator);
            let expected = ["ONE", "two", "THREE"].join(separator);
            assert_eq!(merge(&base, &line, &incoming), Some(expected.into_bytes()));
        }
        assert_eq!(
            merge("a\nb\nc\n", "a\nc\n", "a\nb\nc\nnew\n"),
            Some(b"a\nc\nnew\n".to_vec())
        );
        assert_eq!(
            merge("a\nb\n", "A\nb\n", "a\nB\n"),
            Some(b"A\nB\n".to_vec())
        );
    }

    #[test]
    fn overlapping_edits_and_ambiguous_insertion_boundaries_remain_conflicts() {
        for (base, line, incoming) in [
            ("a\nb\n", "A\nb\n", "other\nb\n"),
            ("a\n", "before\na\n", "A\n"),
            ("a\n", "a\nafter\n", "A\n"),
            ("a\n", "x\na\n", "y\na\n"),
            ("a\nb\n", "", "a\nB\n"),
        ] {
            assert!(
                merge(base, line, incoming).is_none(),
                "{base:?} {line:?} {incoming:?}"
            );
        }
    }

    #[test]
    fn identical_edits_are_applied_once_and_uncontested_edits_reconstruct_the_side() {
        let texts = [
            "",
            "a",
            "a\n",
            "a\nb\n",
            "a\rb\r",
            "a\r\nb",
            "repeat\nrepeat\n",
            "é\n🙂",
        ];
        for base in texts {
            for side in texts {
                assert_eq!(merge(base, side, side), Some(side.as_bytes().to_vec()));
                assert_eq!(merge(base, base, side), Some(side.as_bytes().to_vec()));
                assert_eq!(merge(base, side, base), Some(side.as_bytes().to_vec()));
            }
        }
        assert_eq!(
            merge("a\nb\nc\nd\n", "A\nb\nc\nd\n", "A\nb\nc\nD\n"),
            Some(b"A\nb\nc\nD\n".to_vec())
        );
    }
}
