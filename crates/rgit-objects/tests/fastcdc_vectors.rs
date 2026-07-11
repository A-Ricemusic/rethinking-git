use rgit_objects::{
    Blob, CanonicalObject, FastCdcV0, HashAlgorithm, ObjectId, PolicyId, PolicyRef,
    fastcdc_v0_chunks,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct FastCdcVector {
    name: String,
    input_length: usize,
    input_sha256: String,
    chunk_lengths: Vec<usize>,
    chunk_sha256_ids: Vec<String>,
    blob_sha256_id: String,
    blob_blake3_id: String,
}

fn id(byte: u8) -> ObjectId {
    let mut encoded = vec![0, 0x12, 32];
    encoded.extend([byte; 32]);
    ObjectId::from_bytes(&encoded).unwrap()
}
fn policy() -> PolicyRef {
    PolicyRef {
        policy_id: PolicyId::from_bytes([1; 16]),
        version: id(1),
    }
}
fn forced_max_input() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(rgit_objects::FASTCDC_V0_MAX_SIZE);
    let mut rolling = 0_u64;
    for length in 1..=rgit_objects::FASTCDC_V0_MAX_SIZE {
        if length < rgit_objects::FASTCDC_V0_MIN_SIZE {
            bytes.push(0);
            continue;
        }
        let mask = if length < rgit_objects::FASTCDC_V0_TARGET_SIZE {
            rgit_objects::FASTCDC_V0_EARLY_MASK
        } else {
            rgit_objects::FASTCDC_V0_LATE_MASK
        };
        let (byte, next) = (0_u8..=u8::MAX)
            .find_map(|byte| {
                let next = rolling
                    .rotate_left(1)
                    .wrapping_add(rgit_objects::fastcdc_v0_gear(byte));
                (next & mask != 0).then_some((byte, next))
            })
            .unwrap();
        bytes.push(byte);
        rolling = next;
    }
    bytes
}
fn inputs() -> Vec<(&'static str, Vec<u8>)> {
    let mut state = 0x0123_4567_89ab_cdef_u64;
    let random = (0..10 * 1024 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    vec![
        ("empty", vec![]),
        ("inline-1", vec![0x41]),
        ("inline-65535", vec![0; 65_535]),
        ("inline-65536", vec![0; 65_536]),
        ("chunked-65537", vec![0; 65_537]),
        (
            "profile-min-256kib",
            vec![0; rgit_objects::FASTCDC_V0_MIN_SIZE],
        ),
        (
            "profile-target-1mib",
            vec![0; rgit_objects::FASTCDC_V0_TARGET_SIZE],
        ),
        ("forced-max-boundary-4mib", forced_max_input()),
        ("zeroes-5mib", vec![0; 5 * 1024 * 1024]),
        (
            "ramp-8mib",
            (0..8 * 1024 * 1024).map(|index| index as u8).collect(),
        ),
        ("xorshift-10mib", random),
    ]
}
fn segmented(bytes: &[u8], read_size: usize) -> Vec<Vec<u8>> {
    let mut chunker = FastCdcV0::new();
    let mut chunks = Vec::new();
    for input in bytes.chunks(read_size) {
        chunks.extend(chunker.push(input));
    }
    chunks.extend(chunker.finish());
    chunks
}
fn vectors() -> Vec<FastCdcVector> {
    inputs()
        .into_iter()
        .map(|(name, bytes)| {
            let chunks = fastcdc_v0_chunks(&bytes);
            for read_size in [1, 17, 4093, 65_537] {
                if bytes.len() <= 1024 || read_size != 1 {
                    assert_eq!(
                        segmented(&bytes, read_size),
                        chunks,
                        "read segmentation changed {name}"
                    );
                }
            }
            let built =
                Blob::from_bytes_fastcdc_v0(policy(), &bytes, HashAlgorithm::Sha256).unwrap();
            if bytes.len() > rgit_objects::MAX_INLINE_BLOB_BYTES {
                assert_eq!(
                    built
                        .chunks
                        .iter()
                        .map(|chunk| chunk.bytes.clone())
                        .collect::<Vec<_>>(),
                    chunks
                );
            } else {
                assert!(built.chunks.is_empty());
            }
            FastCdcVector {
                name: name.into(),
                input_length: bytes.len(),
                input_sha256: hex::encode(Sha256::digest(&bytes)),
                chunk_lengths: chunks.iter().map(Vec::len).collect(),
                chunk_sha256_ids: built
                    .chunks
                    .iter()
                    .map(|chunk| chunk.id(HashAlgorithm::Sha256).unwrap().to_string())
                    .collect(),
                blob_sha256_id: built.blob.id(HashAlgorithm::Sha256).unwrap().to_string(),
                blob_blake3_id: built
                    .blob
                    .id(HashAlgorithm::Blake3_256)
                    .unwrap()
                    .to_string(),
            }
        })
        .collect()
}

#[test]
fn fastcdc_profile_zero_known_answers_and_streaming_are_frozen() {
    let actual = vectors();
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/fastcdc-v0.json");
    if std::env::var_os("RGIT_UPDATE_VECTORS").is_some() {
        std::fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(&actual).unwrap()),
        )
        .unwrap();
        return;
    }
    let expected: Vec<FastCdcVector> =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(actual, expected);
}
