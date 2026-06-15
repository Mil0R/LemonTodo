use anyhow::{Context, Result, bail};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine, engine::general_purpose::STANDARD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};

pub const VAULT_KEY_LEN: usize = 32;
pub const XCHACHA20_NONCE_LEN: usize = 24;
pub const KDF_SALT_LEN: usize = 16;

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
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(value: &[u8]) -> Result<Self> {
        if value.len() != VAULT_KEY_LEN {
            bail!(
                "vault key must be {} bytes, got {} bytes",
                VAULT_KEY_LEN,
                value.len()
            );
        }

        let mut key = [0_u8; VAULT_KEY_LEN];
        key.copy_from_slice(value);
        Ok(Self(key))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn as_bytes(&self) -> &[u8; VAULT_KEY_LEN] {
        &self.0
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    pub algorithm: String,
    pub version: u32,
    pub memory_cost_kib: u32,
    pub time_cost: u32,
    pub parallelism: u32,
    pub salt: String,
}

impl KdfParams {
    pub fn generate_interactive() -> Self {
        let mut salt = [0_u8; KDF_SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        Self {
            algorithm: "argon2id".to_owned(),
            version: 19,
            memory_cost_kib: 64 * 1024,
            time_cost: 3,
            parallelism: 1,
            salt: STANDARD.encode(salt),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedVaultKey {
    pub kdf: KdfParams,
    pub envelope: CryptoEnvelope,
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

pub fn wrap_vault_key(
    vault_key: &VaultKey,
    master_password: &str,
    kdf: KdfParams,
) -> Result<EncryptedVaultKey> {
    let wrapping_key = derive_wrapping_key(master_password, &kdf)?;
    let envelope = encrypt(
        &wrapping_key,
        vault_key.as_bytes(),
        b"lemontodo:vault-key:v1",
    )?;
    Ok(EncryptedVaultKey { kdf, envelope })
}

pub fn unwrap_vault_key(encrypted: &EncryptedVaultKey, master_password: &str) -> Result<VaultKey> {
    let wrapping_key = derive_wrapping_key(master_password, &encrypted.kdf)?;
    let plaintext = decrypt(
        &wrapping_key,
        &encrypted.envelope,
        b"lemontodo:vault-key:v1",
    )?;
    VaultKey::from_bytes(&plaintext)
}

fn derive_wrapping_key(master_password: &str, kdf: &KdfParams) -> Result<VaultKey> {
    if kdf.algorithm != "argon2id" {
        bail!("unsupported kdf algorithm {}", kdf.algorithm);
    }
    if kdf.version != 19 {
        bail!("unsupported argon2 version {}", kdf.version);
    }

    let salt = STANDARD
        .decode(&kdf.salt)
        .context("failed to decode kdf salt")?;
    let params = Params::new(
        kdf.memory_cost_kib,
        kdf.time_cost,
        kdf.parallelism,
        Some(VAULT_KEY_LEN),
    )
    .map_err(|error| anyhow::anyhow!("invalid argon2 params: {error}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut output = [0_u8; VAULT_KEY_LEN];
    argon2
        .hash_password_into(master_password.as_bytes(), &salt, &mut output)
        .map_err(|error| anyhow::anyhow!("failed to derive wrapping key: {error}"))?;
    Ok(VaultKey(output))
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

    #[test]
    fn wraps_and_unwraps_vault_key() {
        let vault_key = VaultKey::generate();
        let encrypted = wrap_vault_key(
            &vault_key,
            "correct horse battery staple",
            KdfParams::generate_interactive(),
        )
        .unwrap();

        let unwrapped = unwrap_vault_key(&encrypted, "correct horse battery staple").unwrap();
        assert_eq!(unwrapped, vault_key);
        assert!(unwrap_vault_key(&encrypted, "wrong password").is_err());
    }
}
