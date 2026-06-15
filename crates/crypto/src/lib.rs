use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};

pub const VAULT_KEY_LEN: usize = 32;
pub const XCHACHA20_NONCE_LEN: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultKey([u8; VAULT_KEY_LEN]);

impl VaultKey {
    pub fn generate() -> Self {
        let mut key = [0_u8; VAULT_KEY_LEN];
        OsRng.fill_bytes(&mut key);
        Self(key)
    }

    pub fn from_hex(value: &str) -> Result<Self> {
        let bytes = hex::decode(value.trim()).context("failed to decode vault key hex")?;
        if bytes.len() != VAULT_KEY_LEN {
            bail!(
                "vault key must be {} bytes, got {} bytes",
                VAULT_KEY_LEN,
                bytes.len()
            );
        }

        let mut key = [0_u8; VAULT_KEY_LEN];
        key.copy_from_slice(&bytes);
        Ok(Self(key))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(&self.0.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CryptoEnvelope {
    pub version: u32,
    pub cipher: String,
    pub nonce: String,
    pub ciphertext: String,
}

impl CryptoEnvelope {
    pub const VERSION: u32 = 1;
    pub const CIPHER: &'static str = "xchacha20poly1305";
}

pub fn encrypt(vault_key: &VaultKey, plaintext: &[u8], aad: &[u8]) -> Result<CryptoEnvelope> {
    let mut nonce = [0_u8; XCHACHA20_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);

    let ciphertext = vault_key
        .cipher()
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|error| anyhow::anyhow!("failed to encrypt envelope: {error}"))?;

    Ok(CryptoEnvelope {
        version: CryptoEnvelope::VERSION,
        cipher: CryptoEnvelope::CIPHER.to_owned(),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    })
}

pub fn decrypt(vault_key: &VaultKey, envelope: &CryptoEnvelope, aad: &[u8]) -> Result<Vec<u8>> {
    if envelope.version != CryptoEnvelope::VERSION {
        bail!("unsupported envelope version {}", envelope.version);
    }
    if envelope.cipher != CryptoEnvelope::CIPHER {
        bail!("unsupported cipher {}", envelope.cipher);
    }

    let nonce = STANDARD
        .decode(&envelope.nonce)
        .context("failed to decode nonce")?;
    if nonce.len() != XCHACHA20_NONCE_LEN {
        bail!(
            "nonce must be {} bytes, got {} bytes",
            XCHACHA20_NONCE_LEN,
            nonce.len()
        );
    }
    let ciphertext = STANDARD
        .decode(&envelope.ciphertext)
        .context("failed to decode ciphertext")?;

    vault_key
        .cipher()
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: ciphertext.as_ref(),
                aad,
            },
        )
        .map_err(|error| anyhow::anyhow!("failed to decrypt envelope: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypts_and_decrypts() {
        let key = VaultKey::generate();
        let aad = b"lemontodo:test";
        let envelope = encrypt(&key, b"hello", aad).unwrap();
        let plaintext = decrypt(&key, &envelope, aad).unwrap();
        assert_eq!(plaintext, b"hello");
    }

    #[test]
    fn rejects_wrong_aad() {
        let key = VaultKey::generate();
        let envelope = encrypt(&key, b"hello", b"aad-1").unwrap();
        assert!(decrypt(&key, &envelope, b"aad-2").is_err());
    }
}
