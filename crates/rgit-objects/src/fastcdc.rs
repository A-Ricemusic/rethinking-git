//! Frozen schema-0 FastCDC profile.

use crate::{
    Blob, BlobContent, CanonicalObject, Chunk, ChunkProfile, ChunkRef, HashAlgorithm, ObjectError,
    PolicyRef,
};

pub const FASTCDC_V0_MIN_SIZE: usize = 256 * 1024;
pub const FASTCDC_V0_TARGET_SIZE: usize = 1024 * 1024;
pub const FASTCDC_V0_MAX_SIZE: usize = 4 * 1024 * 1024;
pub const FASTCDC_V0_GEAR_SEED: u64 = 0x7267_6974_6663_6463;
pub const FASTCDC_V0_EARLY_MASK: u64 = (1_u64 << 21) - 1;
pub const FASTCDC_V0_LATE_MASK: u64 = (1_u64 << 19) - 1;

/// Derives one entry of the frozen 256-entry gear table using SplitMix64.
///
/// Defining the complete table by this closed formula avoids platform-specific
/// generated source while producing exactly the same table on every reader.
#[must_use]
pub const fn fastcdc_v0_gear(byte: u8) -> u64 {
    let mut value =
        FASTCDC_V0_GEAR_SEED.wrapping_add((byte as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Incremental FastCDC profile-0 chunker.
///
/// `push` may be called with any segmentation, including empty slices. Completed
/// chunks and final boundaries depend only on the concatenated byte stream.
#[derive(Clone, Debug, Default)]
pub struct FastCdcV0 {
    pending: Vec<u8>,
    rolling: u64,
}

impl FastCdcV0 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: Vec::with_capacity(FASTCDC_V0_TARGET_SIZE),
            rolling: 0,
        }
    }

    pub fn push(&mut self, input: &[u8]) -> Vec<Vec<u8>> {
        let mut completed = Vec::new();
        for &byte in input {
            self.pending.push(byte);
            let length = self.pending.len();
            if length < FASTCDC_V0_MIN_SIZE {
                continue;
            }
            self.rolling = self
                .rolling
                .rotate_left(1)
                .wrapping_add(fastcdc_v0_gear(byte));
            let mask = if length < FASTCDC_V0_TARGET_SIZE {
                FASTCDC_V0_EARLY_MASK
            } else {
                FASTCDC_V0_LATE_MASK
            };
            if self.rolling & mask == 0 || length == FASTCDC_V0_MAX_SIZE {
                completed.push(std::mem::replace(
                    &mut self.pending,
                    Vec::with_capacity(FASTCDC_V0_TARGET_SIZE),
                ));
                self.rolling = 0;
            }
        }
        completed
    }

    /// Completes the stream. Empty input has no chunks; every nonempty input has
    /// one final nonempty chunk even when shorter than the profile minimum.
    #[must_use]
    pub fn finish(mut self) -> Vec<Vec<u8>> {
        if self.pending.is_empty() {
            Vec::new()
        } else {
            vec![std::mem::take(&mut self.pending)]
        }
    }
}

#[must_use]
pub fn fastcdc_v0_chunks(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut chunker = FastCdcV0::new();
    let mut chunks = chunker.push(bytes);
    chunks.extend(chunker.finish());
    chunks
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkedBlob {
    pub blob: Blob,
    pub chunks: Vec<Chunk>,
}

impl Blob {
    /// Constructs the one canonical schema-0 representation for complete bytes.
    pub fn from_bytes_fastcdc_v0(
        policy_ref: PolicyRef,
        bytes: &[u8],
        hash_algorithm: HashAlgorithm,
    ) -> Result<ChunkedBlob, ObjectError> {
        if bytes.len() <= crate::MAX_INLINE_BLOB_BYTES {
            return Ok(ChunkedBlob {
                blob: Self {
                    policy_ref,
                    byte_length: bytes.len() as u64,
                    content: BlobContent::Inline(bytes.to_vec()),
                    chunk_profile: None,
                    content_hint: None,
                },
                chunks: Vec::new(),
            });
        }
        let chunks = fastcdc_v0_chunks(bytes)
            .into_iter()
            .map(|bytes| Chunk {
                policy_ref: policy_ref.clone(),
                bytes,
            })
            .collect::<Vec<_>>();
        let references = chunks
            .iter()
            .map(|chunk| {
                Ok(ChunkRef {
                    id: chunk.id(hash_algorithm)?,
                    plaintext_length: chunk.bytes.len() as u64,
                })
            })
            .collect::<Result<Vec<_>, ObjectError>>()?;
        Ok(ChunkedBlob {
            blob: Self {
                policy_ref,
                byte_length: bytes.len() as u64,
                content: BlobContent::Chunks(references),
                chunk_profile: Some(ChunkProfile::fastcdc_v0()),
                content_hint: None,
            },
            chunks,
        })
    }
}
