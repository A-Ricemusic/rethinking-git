//! Conservative cross-platform alias checks before materializing saved paths.
use anyhow::{bail, Result};
use std::{collections::BTreeMap, fs, path::Path};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

fn fold(value: &str) -> String {
    value.case_fold().collect::<String>().nfc().collect()
}

/// Compare every prefix, not just complete filenames: Src/a and src/b alias too.
pub(crate) fn validate<'a>(paths: impl IntoIterator<Item = &'a str>) -> Result<()> {
    let mut prefixes = BTreeMap::new();
    for path in paths {
        super::transaction::validate_working_key(path)?;
        let mut prefix = String::new();
        for part in path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if let Some(previous) = prefixes.insert(fold(&prefix), prefix.clone()) {
                if previous != prefix {
                    bail!("checkout paths alias under portable filename comparison");
                }
            }
        }
    }
    Ok(())
}

/// Refuse existing alternate spellings, including untracked parent directories.
/// Read each directory only once for a whole checkout/recovery set.
pub(crate) fn validate_existing<'a>(
    root: &Path,
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    let mut directories = BTreeMap::new();
    for path in paths {
        super::transaction::validate_working_key(path)?;
        let mut parent = root.to_path_buf();
        let parts: Vec<_> = path.split('/').collect();
        for (index, part) in parts.iter().enumerate() {
            let names = match directories.entry(parent.clone()) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
                    for entry in fs::read_dir(&parent)? {
                        if let Some(name) = entry?.file_name().to_str() {
                            names.entry(fold(name)).or_default().push(name.to_owned());
                        }
                    }
                    entry.insert(names)
                }
            };
            if names
                .get(&fold(part))
                .is_some_and(|spellings| spellings.iter().any(|name| name != part))
            {
                bail!("checkout path aliases an existing filesystem entry");
            }
            parent.push(part);
            if index + 1 < parts.len() {
                match fs::symlink_metadata(&parent) {
                    Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                    Ok(_) => bail!("checkout path has an unsafe parent"),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_case_normalization_expansion_and_directory_aliases() {
        for paths in [
            ["README", "readme"],
            ["Src/a", "src/b"],
            ["é", "e\u{301}"],
            ["Straße/a", "STRASSE/b"],
            ["FOO", "foo/child"],
        ] {
            assert!(validate(paths).is_err(), "{paths:?}");
        }
        validate(["src/a", "src/b", "src/a", "README"]).unwrap();
    }
}
