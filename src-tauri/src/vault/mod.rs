use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use thiserror::Error;
use zeroize::Zeroize;

const MAGIC: &[u8; 8] = b"SIFTVLT1";
const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 12;
const KEY_BYTES: usize = 32;
const MEMORY_KIB: u32 = 64 * 1024;
const ITERATIONS: u32 = 3;

#[derive(Debug, Error)]
pub enum VaultCryptoError {
    #[error("The vault envelope is malformed or uses an unsupported version.")]
    InvalidEnvelope,
    #[error("A cryptographic operation could not be completed.")]
    Crypto,
}

/// Encrypt a payload using an Argon2id-derived key and a fresh AES-256-GCM nonce.
/// The result is a versioned envelope; it contains no plaintext filename or metadata.
pub fn encrypt_payload(passphrase: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, VaultCryptoError> {
    if passphrase.is_empty() { return Err(VaultCryptoError::Crypto); }
    let mut salt = [0_u8; SALT_BYTES];
    let mut nonce = [0_u8; NONCE_BYTES];
    getrandom::getrandom(&mut salt).map_err(|_| VaultCryptoError::Crypto)?;
    getrandom::getrandom(&mut nonce).map_err(|_| VaultCryptoError::Crypto)?;
    let mut key = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| VaultCryptoError::Crypto)?;
    let encrypted_result = cipher.encrypt(Nonce::from_slice(&nonce), plaintext);
    drop(cipher);
    key.zeroize();
    let encrypted = encrypted_result.map_err(|_| VaultCryptoError::Crypto)?;
    let mut envelope = Vec::with_capacity(MAGIC.len() + SALT_BYTES + NONCE_BYTES + encrypted.len());
    envelope.extend_from_slice(MAGIC);
    envelope.extend_from_slice(&salt);
    envelope.extend_from_slice(&nonce);
    envelope.extend_from_slice(&encrypted);
    Ok(envelope)
}

/// Authenticate and decrypt one complete envelope. AES-GCM authentication failure
/// intentionally does not distinguish an incorrect passphrase from altered data.
pub fn decrypt_payload(passphrase: &[u8], envelope: &[u8]) -> Result<Vec<u8>, VaultCryptoError> {
    let header_bytes = MAGIC.len() + SALT_BYTES + NONCE_BYTES;
    if envelope.len() < header_bytes + 16 || &envelope[..MAGIC.len()] != MAGIC {
        return Err(VaultCryptoError::InvalidEnvelope);
    }
    if passphrase.is_empty() { return Err(VaultCryptoError::Crypto); }
    let salt_start = MAGIC.len();
    let nonce_start = salt_start + SALT_BYTES;
    let data_start = nonce_start + NONCE_BYTES;
    let salt = &envelope[salt_start..nonce_start];
    let nonce = &envelope[nonce_start..data_start];
    let mut key = derive_key(passphrase, salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| VaultCryptoError::Crypto)?;
    let decrypted = cipher.decrypt(Nonce::from_slice(nonce), &envelope[data_start..]);
    drop(cipher);
    key.zeroize();
    decrypted.map_err(|_| VaultCryptoError::Crypto)
}

fn derive_key(passphrase: &[u8], salt: &[u8]) -> Result<[u8; KEY_BYTES], VaultCryptoError> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, 1, Some(KEY_BYTES)).map_err(|_| VaultCryptoError::Crypto)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0_u8; KEY_BYTES];
    argon.hash_password_into(passphrase, salt, &mut key).map_err(|_| VaultCryptoError::Crypto)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_payload_round_trips_and_authenticates() {
        let encrypted = encrypt_payload(b"test passphrase", b"private file bytes").expect("encryption succeeds");
        assert_ne!(&encrypted[header_len()..], b"private file bytes");
        assert_eq!(decrypt_payload(b"test passphrase", &encrypted).expect("decryption succeeds"), b"private file bytes");
        assert!(decrypt_payload(b"wrong passphrase", &encrypted).is_err());
    }

    #[test]
    fn rejects_truncated_or_wrong_version_envelopes() {
        assert!(matches!(decrypt_payload(b"pass", b"short"), Err(VaultCryptoError::InvalidEnvelope)));
    }

    fn header_len() -> usize { MAGIC.len() + SALT_BYTES + NONCE_BYTES }
}
