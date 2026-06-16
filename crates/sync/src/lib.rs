use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use lemontodo_core::Operation;
use lemontodo_crypto::{CryptoEnvelope, EncryptedVaultKey, VaultKey, decrypt, encrypt};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPack {
    pub version: u32,
    pub device_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub objects: Vec<EncryptedSyncObject>,
}

impl SyncPack {
    pub const VERSION: u32 = PROTOCOL_VERSION;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub protocol_version: u32,
    pub capabilities: Vec<ServerCapability>,
    pub auth: ServerAuthInfo,
    pub limits: ServerLimits,
}

impl ServerInfo {
    pub fn minimal() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            capabilities: vec![
                ServerCapability::ObjectSync,
                ServerCapability::BatchPush,
                ServerCapability::CursorPull,
                ServerCapability::PasswordAuth,
            ],
            auth: ServerAuthInfo {
                password_auth: true,
                registration_allowed: false,
            },
            limits: ServerLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerCapability {
    ObjectSync,
    CursorPull,
    BatchPush,
    PasswordAuth,
    AccountRegistration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ServerAuthInfo {
    pub password_auth: bool,
    pub registration_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerLimits {
    pub max_push_objects: u32,
    pub max_pull_objects: u32,
    pub max_object_bytes: u32,
}

impl Default for ServerLimits {
    fn default() -> Self {
        Self {
            max_push_objects: 500,
            max_pull_objects: 500,
            max_object_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRequest {
    pub protocol_version: u32,
    pub device_id: Uuid,
    pub access_token: String,
    pub base_cursor: Option<String>,
    pub objects: Vec<EncryptedSyncObject>,
}

impl PushRequest {
    pub fn from_pack(access_token: String, base_cursor: Option<String>, pack: SyncPack) -> Self {
        Self {
            protocol_version: pack.version,
            device_id: pack.device_id,
            access_token,
            base_cursor,
            objects: pack.objects,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushResponse {
    pub protocol_version: u32,
    pub accepted: Vec<AcceptedSyncObject>,
    pub rejected: Vec<RejectedSyncObject>,
    pub cursor: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedSyncObject {
    pub operation_id: Uuid,
    pub object_id: Uuid,
    pub object_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedSyncObject {
    pub operation_id: Uuid,
    pub reason: RejectionReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    Duplicate,
    PayloadTooLarge,
    InvalidEnvelope,
    UnsupportedProtocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub protocol_version: u32,
    pub device_id: Uuid,
    pub access_token: String,
    pub cursor: Option<String>,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullResponse {
    pub protocol_version: u32,
    pub cursor: String,
    pub has_more: bool,
    pub objects: Vec<EncryptedSyncObject>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub auth_hash: String,
    pub encrypted_vault_key: EncryptedVaultKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub user_id: Uuid,
    pub email: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub auth_hash: String,
    pub device_id: Uuid,
    pub device_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginResponse {
    pub user_id: Uuid,
    pub email: String,
    pub access_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutRequest {
    pub access_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutResponse {
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountStatusResponse {
    pub user_id: Uuid,
    pub email: String,
    pub is_admin: bool,
    pub has_vault_key: bool,
    pub user_created_at: DateTime<Utc>,
    pub session_created_at: DateTime<Utc>,
    pub session_last_used_at: DateTime<Utc>,
    pub session_device_id: Option<Uuid>,
    pub session_device_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub current: bool,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
    pub device_id: Option<Uuid>,
    pub device_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionsResponse {
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeSessionRequest {
    pub access_token: String,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeSessionResponse {
    pub revoked: bool,
    pub current: bool,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutVaultMetadataRequest {
    pub access_token: String,
    pub encrypted_vault_key: EncryptedVaultKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultMetadataResponse {
    pub has_vault_key: bool,
    pub encrypted_vault_key: Option<EncryptedVaultKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedSyncObject {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub device_id: Uuid,
    pub object_id: Uuid,
    pub object_revision: i64,
    pub object_type: String,
    pub operation_type: String,
    pub created_at: DateTime<Utc>,
    pub envelope: CryptoEnvelope,
}

pub fn pack_operations(
    vault_key: &VaultKey,
    device_id: Uuid,
    operations: &[Operation],
) -> Result<SyncPack> {
    let objects = operations
        .iter()
        .map(|operation| pack_operation(vault_key, operation))
        .collect::<Result<Vec<_>>>()?;

    Ok(SyncPack {
        version: SyncPack::VERSION,
        device_id,
        created_at: Utc::now(),
        objects,
    })
}

pub fn unpack_operation(vault_key: &VaultKey, object: &EncryptedSyncObject) -> Result<Operation> {
    let plaintext = decrypt(
        vault_key,
        &object.envelope,
        aad_for_object(object).as_bytes(),
    )?;
    serde_json::from_slice(&plaintext).context("failed to deserialize operation")
}

fn pack_operation(vault_key: &VaultKey, operation: &Operation) -> Result<EncryptedSyncObject> {
    let object = EncryptedSyncObject {
        id: Uuid::new_v4(),
        operation_id: operation.id,
        device_id: operation.device_id,
        object_id: operation.object_id,
        object_revision: operation.object_revision,
        object_type: operation.object_type.as_str().to_owned(),
        operation_type: operation.operation_type.as_str().to_owned(),
        created_at: Utc::now(),
        envelope: CryptoEnvelope {
            version: CryptoEnvelope::VERSION,
            cipher: CryptoEnvelope::CIPHER.to_owned(),
            nonce: String::new(),
            ciphertext: String::new(),
        },
    };
    let plaintext = serde_json::to_vec(operation).context("failed to serialize operation")?;
    let envelope = encrypt(vault_key, &plaintext, aad_for_object(&object).as_bytes())?;

    Ok(EncryptedSyncObject { envelope, ..object })
}

fn aad_for_object(object: &EncryptedSyncObject) -> String {
    format!(
        "lemontodo-sync:v1:{}:{}:{}:{}:{}:{}",
        object.operation_id,
        object.device_id,
        object.object_id,
        object.object_revision,
        object.object_type,
        object.operation_type
    )
}

#[cfg(test)]
mod tests {
    use lemontodo_core::{ObjectType, OperationType};
    use serde_json::json;

    use super::*;

    #[test]
    fn packs_and_unpacks_operation() {
        let key = VaultKey::generate();
        let operation = Operation {
            id: Uuid::new_v4(),
            device_id: Uuid::new_v4(),
            object_id: Uuid::new_v4(),
            object_revision: 1,
            object_type: ObjectType::Task,
            operation_type: OperationType::Create,
            payload: json!({ "title": "test" }),
            created_at: Utc::now(),
            synced_at: None,
        };

        let pack =
            pack_operations(&key, operation.device_id, std::slice::from_ref(&operation)).unwrap();
        assert_eq!(pack.device_id, operation.device_id);
        assert_eq!(pack.objects.len(), 1);
        assert_eq!(pack.objects[0].device_id, operation.device_id);
        assert_eq!(pack.objects[0].object_revision, operation.object_revision);
        let unpacked = unpack_operation(&key, &pack.objects[0]).unwrap();
        assert_eq!(unpacked.id, operation.id);
        assert_eq!(unpacked.payload, operation.payload);
    }

    #[test]
    fn builds_push_request_from_sync_pack() {
        let key = VaultKey::generate();
        let operation = Operation {
            id: Uuid::new_v4(),
            device_id: Uuid::new_v4(),
            object_id: Uuid::new_v4(),
            object_revision: 3,
            object_type: ObjectType::Task,
            operation_type: OperationType::Update,
            payload: json!({ "title": "updated" }),
            created_at: Utc::now(),
            synced_at: None,
        };

        let pack =
            pack_operations(&key, operation.device_id, std::slice::from_ref(&operation)).unwrap();
        let request =
            PushRequest::from_pack("token-1".to_owned(), Some("cursor-1".to_owned()), pack);

        assert_eq!(request.protocol_version, PROTOCOL_VERSION);
        assert_eq!(request.device_id, operation.device_id);
        assert_eq!(request.access_token, "token-1");
        assert_eq!(request.base_cursor, Some("cursor-1".to_owned()));
        assert_eq!(request.objects.len(), 1);
        assert_eq!(request.objects[0].operation_id, operation.id);
    }

    #[test]
    fn serializes_protocol_dtos_with_stable_names() {
        let server_info = ServerInfo::minimal();
        let json = serde_json::to_value(&server_info).unwrap();
        assert_eq!(json["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(json["capabilities"][0], "object_sync");
        assert_eq!(json["capabilities"][1], "batch_push");
        assert_eq!(json["capabilities"][2], "cursor_pull");
        assert_eq!(json["limits"]["max_push_objects"], 500);

        let pull = PullRequest {
            protocol_version: PROTOCOL_VERSION,
            device_id: Uuid::nil(),
            access_token: "token-2".to_owned(),
            cursor: Some("cursor-2".to_owned()),
            limit: 100,
        };
        let json = serde_json::to_value(&pull).unwrap();
        assert_eq!(json["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(json["device_id"], Uuid::nil().to_string());
        assert_eq!(json["access_token"], "token-2");
        assert_eq!(json["cursor"], "cursor-2");
        assert_eq!(json["limit"], 100);

        let login = LoginRequest {
            email: "user@example.com".to_owned(),
            auth_hash: "auth-hash".to_owned(),
            device_id: Uuid::nil(),
            device_name: "workstation".to_owned(),
        };
        let json = serde_json::to_value(&login).unwrap();
        assert_eq!(json["auth_hash"], "auth-hash");
        assert_eq!(json["device_id"], Uuid::nil().to_string());
        assert_eq!(json["device_name"], "workstation");

        let rejection = RejectedSyncObject {
            operation_id: Uuid::nil(),
            reason: RejectionReason::Duplicate,
        };
        let json = serde_json::to_value(&rejection).unwrap();
        assert_eq!(json["reason"], "duplicate");

        let account = AccountStatusResponse {
            user_id: Uuid::nil(),
            email: "user@example.com".to_owned(),
            is_admin: false,
            has_vault_key: true,
            user_created_at: Utc::now(),
            session_created_at: Utc::now(),
            session_last_used_at: Utc::now(),
            session_device_id: Some(Uuid::nil()),
            session_device_name: Some("workstation".to_owned()),
        };
        let json = serde_json::to_value(&account).unwrap();
        assert_eq!(json["user_id"], Uuid::nil().to_string());
        assert_eq!(json["email"], "user@example.com");
        assert_eq!(json["is_admin"], false);
        assert_eq!(json["has_vault_key"], true);
        assert_eq!(json["session_device_id"], Uuid::nil().to_string());
        assert_eq!(json["session_device_name"], "workstation");
        assert!(json.get("session_last_used_at").is_some());

        let sessions = SessionsResponse {
            sessions: vec![SessionInfo {
                session_id: "abc12345".to_owned(),
                current: true,
                created_at: Utc::now(),
                last_used_at: Utc::now(),
                device_id: Some(Uuid::nil()),
                device_name: Some("workstation".to_owned()),
            }],
        };
        let json = serde_json::to_value(&sessions).unwrap();
        assert_eq!(json["sessions"][0]["session_id"], "abc12345");
        assert_eq!(json["sessions"][0]["current"], true);
        assert_eq!(json["sessions"][0]["device_id"], Uuid::nil().to_string());
        assert_eq!(json["sessions"][0]["device_name"], "workstation");

        let revoke = RevokeSessionRequest {
            access_token: "token-3".to_owned(),
            session_id: "abc12345".to_owned(),
        };
        let json = serde_json::to_value(&revoke).unwrap();
        assert_eq!(json["access_token"], "token-3");
        assert_eq!(json["session_id"], "abc12345");

        let revoke = RevokeSessionResponse {
            revoked: true,
            current: false,
            session_id: "abc12345".to_owned(),
        };
        let json = serde_json::to_value(&revoke).unwrap();
        assert_eq!(json["revoked"], true);
        assert_eq!(json["current"], false);
        assert_eq!(json["session_id"], "abc12345");
    }
}
