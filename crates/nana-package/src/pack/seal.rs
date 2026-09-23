//! XChaCha20-Poly1305 sealing of block records and TOCs.
//!
//! A sealed record is `nonce(24) ‖ ciphertext ‖ tag(16)`. The 192-bit nonce
//! is drawn at random by the packager for every freshly sealed record, which
//! is safe for any realistic number of records under one key. Reusing an
//! existing record byte-for-byte (unchanged input, same key generation) is
//! the same (key, nonce, plaintext) triple, not nonce reuse. A record may be
//! reused only while every input of its associated data is unchanged (pack,
//! entry key, block index, key generation, plaintext length, flags); a
//! renamed entry or a new key generation is resealed with a fresh nonce.
//!
//! [`seal`] takes the nonce as an argument so this crate needs no RNG; only
//! the build-side packager calls it.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use super::format::{NONCE_LEN, SEAL_OVERHEAD};
use crate::keys::ContentKey;

fn cipher(key: &ContentKey) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(key.as_bytes().into())
}

/// Seal `plaintext` under `key` with an explicit `nonce`. The caller must
/// never pass the same nonce for different plaintexts under one key.
pub fn seal(key: &ContentKey, nonce: &[u8; NONCE_LEN], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let ciphertext = cipher(key)
        .encrypt(
            &XNonce::from(*nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("XChaCha20-Poly1305 encryption of an in-memory buffer cannot fail");
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ciphertext);
    out
}

/// Authenticate and decrypt a sealed record. `None` on any failure; the
/// caller never sees unauthenticated plaintext.
pub fn open(key: &ContentKey, aad: &[u8], record: &[u8]) -> Option<Vec<u8>> {
    if record.len() < SEAL_OVERHEAD {
        return None;
    }
    let (nonce, ciphertext) = record.split_at(NONCE_LEN);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    nonce_bytes.copy_from_slice(nonce);
    cipher(key)
        .decrypt(
            &XNonce::from(nonce_bytes),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_round_trip_and_tamper() {
        let key = ContentKey::from_bytes([9; 32]);
        let record = seal(&key, &[1; 24], b"aad", b"hello");
        assert_eq!(record.len(), 5 + SEAL_OVERHEAD);
        assert_eq!(open(&key, b"aad", &record).unwrap(), b"hello");
        assert!(open(&key, b"other", &record).is_none());
        assert!(open(&ContentKey::from_bytes([8; 32]), b"aad", &record).is_none());
        let mut flipped = record.clone();
        flipped[30] ^= 1;
        assert!(open(&key, b"aad", &flipped).is_none());
        assert!(open(&key, b"aad", &record[..10]).is_none());
    }
}
