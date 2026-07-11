//! Deterministic, dependency-free repository fixture generation.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub struct FixtureProfile {
    pub name: &'static str,
    pub files: usize,
    pub bytes_per_file: usize,
    pub directories: usize,
}

pub const PROFILES: [FixtureProfile; 4] = [
    FixtureProfile {
        name: "small",
        files: 128,
        bytes_per_file: 2 * 1024,
        directories: 16,
    },
    FixtureProfile {
        name: "medium",
        files: 4_096,
        bytes_per_file: 4 * 1024,
        directories: 256,
    },
    FixtureProfile {
        name: "linux-scale",
        files: 80_000,
        bytes_per_file: 8 * 1024,
        directories: 4_000,
    },
    FixtureProfile {
        name: "monorepo",
        files: 250_000,
        bytes_per_file: 4 * 1024,
        directories: 10_000,
    },
];

pub fn named_profile(name: &str) -> Option<FixtureProfile> {
    PROFILES
        .iter()
        .copied()
        .find(|profile| profile.name == name)
}

pub fn generate(root: &Path, profile: FixtureProfile) -> io::Result<u64> {
    if profile.directories == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directories must be non-zero",
        ));
    }
    fs::create_dir_all(root)?;
    let mut written = 0_u64;
    for index in 0..profile.files {
        let directory = root
            .join("src")
            .join(format!("d{:05}", index % profile.directories));
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!("file_{index:08}.txt"));
        let mut file = BufWriter::new(File::create(path)?);
        let header = format!("fixture={} index={index:08}\n", profile.name);
        let header = &header.as_bytes()[..header.len().min(profile.bytes_per_file)];
        file.write_all(header)?;
        written += header.len() as u64;
        let remaining = profile.bytes_per_file - header.len();
        let pattern = format!("{:016x}", deterministic_word(index as u64));
        let body: Vec<u8> = pattern.bytes().cycle().take(remaining).collect();
        file.write_all(&body)?;
        written += remaining as u64;
    }
    Ok(written)
}

pub fn custom(
    name: &'static str,
    files: usize,
    bytes_per_file: usize,
    directories: usize,
) -> FixtureProfile {
    FixtureProfile {
        name,
        files,
        bytes_per_file,
        directories,
    }
}

fn deterministic_word(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn generation_is_byte_for_byte_reproducible() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let left = std::env::temp_dir().join(format!("rgit-fixture-left-{nonce}"));
        let right = std::env::temp_dir().join(format!("rgit-fixture-right-{nonce}"));
        let profile = custom("test", 3, 128, 2);
        assert_eq!(generate(&left, profile).unwrap(), 384);
        assert_eq!(generate(&right, profile).unwrap(), 384);
        for index in 0..profile.files {
            let relative = Path::new("src")
                .join(format!("d{:05}", index % profile.directories))
                .join(format!("file_{index:08}.txt"));
            assert_eq!(
                fs::read(left.join(&relative)).unwrap(),
                fs::read(right.join(relative)).unwrap()
            );
        }
        fs::remove_dir_all(left).unwrap();
        fs::remove_dir_all(right).unwrap();
    }
}
