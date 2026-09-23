//! Steam-like delta estimate between two builds of a file.
//!
//! SteamPipe splits files into chunks of about 1 MiB and ships the chunks
//! whose content changed. This models that with fixed, aligned 1 MiB chunks:
//! a proxy, not SteamPipe itself (whose chunking may detect some shifted
//! data). CI gates on these counts; real patch sizes need a steamcmd preview
//! build (see `docs/packaging.md`).

use serde::Serialize;

pub const STEAM_CHUNK: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Delta {
    pub old_bytes: u64,
    pub new_bytes: u64,
    /// Bytes of the new file that differ from the old one at the same
    /// offset (bytes past the old end count as changed).
    pub changed_bytes: u64,
    /// Chunks of the new file that differ from the old file's chunk at the
    /// same index.
    pub changed_chunks: u64,
    pub total_chunks: u64,
    /// Bytes a client downloads: the length of every changed chunk.
    pub steam_delta_bytes: u64,
}

pub fn delta(old: &[u8], new: &[u8], chunk: usize) -> Delta {
    let mut result = Delta {
        old_bytes: old.len() as u64,
        new_bytes: new.len() as u64,
        ..Delta::default()
    };
    for (index, new_chunk) in new.chunks(chunk).enumerate() {
        let start = index * chunk;
        let old_chunk = old
            .get(start..(start + chunk).min(old.len()))
            .unwrap_or(&[]);
        result.total_chunks += 1;
        let same_prefix = new_chunk
            .iter()
            .zip(old_chunk)
            .filter(|(a, b)| a != b)
            .count() as u64;
        let tail = new_chunk.len().saturating_sub(old_chunk.len()) as u64;
        let changed = same_prefix + tail;
        result.changed_bytes += changed;
        if changed > 0 || old_chunk.len() != new_chunk.len() {
            result.changed_chunks += 1;
            result.steam_delta_bytes += new_chunk.len() as u64;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_changed_chunks_and_bytes() {
        let old = vec![0u8; 10];
        let mut new = old.clone();
        new[5] = 1;
        new.extend_from_slice(&[9, 9]);
        let d = delta(&old, &new, 4);
        assert_eq!(d.total_chunks, 3);
        assert_eq!(d.changed_bytes, 3);
        // chunk 1 (byte 5) and chunk 2 (grew from 2 to 4 bytes).
        assert_eq!(d.changed_chunks, 2);
        assert_eq!(d.steam_delta_bytes, 8);
        assert_eq!(delta(&old, &old, 4).changed_chunks, 0);
    }
}
