use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use lemontodo_core::Operation;
use lemontodo_crypto::{CryptoEnvelope, VaultKey, decrypt, encrypt};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPack {
    pub version: u32,
    pub device_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub objects: Vec<EncryptedSyncObject>,
}

impl SyncPack {
    pub const VERSION: u32 = 1;
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
}
