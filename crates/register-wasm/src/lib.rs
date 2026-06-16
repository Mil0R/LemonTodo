use lemontodo_crypto::{KdfParams, VaultKey, derive_auth_hash, wrap_vault_key};
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

fn to_js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
