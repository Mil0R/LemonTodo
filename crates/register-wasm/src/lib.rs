use lemontodo_crypto::{
    CryptoEnvelope, EncryptedVaultKey, KdfParams, VaultKey, decrypt, derive_auth_hash, encrypt,
    unwrap_vault_key, wrap_vault_key,
};
use lemontodo_sync::{LoginRequest, RegisterRequest};
use uuid::Uuid;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn build_register_request(email: &str, master_password: &str) -> Result<String, JsValue> {
    let vault_key = VaultKey::generate();
    let encrypted_vault_key = wrap_vault_key(
        &vault_key,
        master_password,
        KdfParams::generate_interactive(),
    )
    .map_err(to_js_error)?;
    let request = RegisterRequest {
        email: email.trim().to_owned(),
        auth_hash: derive_auth_hash(email, master_password).map_err(to_js_error)?,
        encrypted_vault_key,
    };
    serde_json::to_string(&request).map_err(to_js_error)
}

#[wasm_bindgen]
pub fn build_login_request(
    email: &str,
    master_password: &str,
    device_name: &str,
) -> Result<String, JsValue> {
    let normalized_email = email.trim().to_owned();
    let normalized_device_name = match device_name.trim() {
        "" => "browser".to_owned(),
        value => value.to_owned(),
    };
    let request = LoginRequest {
        email: normalized_email,
        auth_hash: derive_auth_hash(email, master_password).map_err(to_js_error)?,
        device_id: Uuid::new_v4(),
        device_name: normalized_device_name,
    };
    serde_json::to_string(&request).map_err(to_js_error)
}

#[wasm_bindgen]
pub fn unwrap_vault_key_hex(
    encrypted_vault_key_json: &str,
    master_password: &str,
) -> Result<String, JsValue> {
    let encrypted: EncryptedVaultKey =
        serde_json::from_str(encrypted_vault_key_json).map_err(to_js_error)?;
    let vault_key = unwrap_vault_key(&encrypted, master_password).map_err(to_js_error)?;
    Ok(vault_key.to_hex())
}

#[wasm_bindgen]
pub fn encrypt_payload(vault_key_hex: &str, plaintext: &str, aad: &str) -> Result<String, JsValue> {
    let vault_key = VaultKey::from_hex(vault_key_hex).map_err(to_js_error)?;
    let envelope =
        encrypt(&vault_key, plaintext.as_bytes(), aad.as_bytes()).map_err(to_js_error)?;
    serde_json::to_string(&envelope).map_err(to_js_error)
}

#[wasm_bindgen]
pub fn decrypt_payload(
    vault_key_hex: &str,
    envelope_json: &str,
    aad: &str,
) -> Result<String, JsValue> {
    let vault_key = VaultKey::from_hex(vault_key_hex).map_err(to_js_error)?;
    let envelope: CryptoEnvelope = serde_json::from_str(envelope_json).map_err(to_js_error)?;
    let plaintext = decrypt(&vault_key, &envelope, aad.as_bytes()).map_err(to_js_error)?;
    String::from_utf8(plaintext).map_err(to_js_error)
}

fn to_js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
