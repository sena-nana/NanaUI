//! Artifact cache: stored records of entries already encoded, keyed by
//! everything that determines them. Lets a build without the previous
//! package (or a pack whose layout was compacted) still reuse bytes, so an
//! unchanged resource is not re-encrypted under a fresh nonce.
//!
//! Entries are verified by the builder before reuse (`decode_records`); a
//! corrupted or poisoned cache entry costs a re-encode, never a bad pack.
//!
//! Layout: `<dir>/nrpack-v1/<2 hex>/<64 hex>.rec` =
//! `"NRC1" ‖ block_count u32 ‖ block records (32 B each) ‖ records`.

use std::path::{Path, PathBuf};

use nana_package::KeyId;
use nana_package::pack::format::{BLOCK_LEN, BlockRecord};

const MAGIC: &[u8; 4] = b"NRC1";

#[derive(Debug, Clone)]
pub struct ArtifactCache {
    dir: PathBuf,
}

impl ArtifactCache {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.join("nrpack-v1"),
        }
    }

    /// Everything that determines an entry's stored records. `zstd_level` is
    /// `None` for stored (uncompressed) blocks.
    pub fn key(
        pack_id: &[u8; 16],
        entry_key: &str,
        plain_hash: &[u8; 32],
        zstd_level: Option<i32>,
        block_size: u32,
        key_id: KeyId,
        key_generation: u32,
    ) -> [u8; 32] {
        let (codec, level) = match zstd_level {
            Some(level) => (1u8, level),
            None => (0u8, 0),
        };
        nana_package::hash::derive(
            "nana.nrpack.cache.v1",
            &[
                pack_id,
                entry_key.as_bytes(),
                plain_hash,
                &[codec],
                &level.to_le_bytes(),
                &block_size.to_le_bytes(),
                &key_id.0,
                &key_generation.to_le_bytes(),
            ],
        )
    }

    fn path(&self, key: &[u8; 32]) -> PathBuf {
        let hex = nana_package::to_hex(key);
        self.dir.join(&hex[..2]).join(format!("{hex}.rec"))
    }

    pub fn get(&self, key: &[u8; 32]) -> Option<(Vec<BlockRecord>, Vec<u8>)> {
        let bytes = std::fs::read(self.path(key)).ok()?;
        if bytes.len() < 8 || &bytes[..4] != MAGIC {
            return None;
        }
        let count = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
        let table_end = 8usize.checked_add(count.checked_mul(BLOCK_LEN)?)?;
        let table = bytes.get(8..table_end)?;
        let blocks: Vec<BlockRecord> = table.chunks(BLOCK_LEN).map(BlockRecord::decode).collect();
        let stored: usize = blocks.iter().map(|b| b.stored_len as usize).sum();
        let records = bytes.get(table_end..)?;
        (records.len() == stored).then(|| (blocks, records.to_vec()))
    }

    pub fn put(
        &self,
        key: &[u8; 32],
        blocks: &[BlockRecord],
        records: &[u8],
    ) -> Result<(), String> {
        let path = self.path(key);
        let parent = path.parent().expect("cache path has a parent");
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create cache {}: {error}", parent.display()))?;
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
        for block in blocks {
            block.encode(&mut bytes);
        }
        bytes.extend_from_slice(records);
        let temp = path.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&temp, &bytes)
            .and_then(|()| std::fs::rename(&temp, &path))
            .map_err(|error| format!("cannot write cache {}: {error}", path.display()))
    }
}
