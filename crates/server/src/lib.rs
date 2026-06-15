use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::Utc;
use lemontodo_sync::{
    AcceptedSyncObject, EncryptedSyncObject, PROTOCOL_VERSION, PushRequest, PushResponse,
    RejectedSyncObject, RejectionReason, ServerInfo,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8787;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub database_path: PathBuf,
    pub allow_registration: bool,
    pub admin_email: Option<String>,
    pub admin_password: Option<String>,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Result<Self> {
        let host = lookup("LEMONTODO_SERVER_HOST").unwrap_or_else(|| DEFAULT_HOST.to_owned());
        let port = lookup("LEMONTODO_SERVER_PORT")
            .map(|value| {
                value
                    .parse::<u16>()
                    .with_context(|| format!("invalid LEMONTODO_SERVER_PORT: {value}"))
            })
            .transpose()?
            .unwrap_or(DEFAULT_PORT);
        let database_path = lookup("LEMONTODO_SERVER_DB")
            .map(PathBuf::from)
            .unwrap_or_else(default_database_path);
        let allow_registration = lookup("LEMONTODO_ALLOW_REGISTRATION")
            .map(|value| parse_bool(&value, "LEMONTODO_ALLOW_REGISTRATION"))
            .transpose()?
            .unwrap_or(false);
        let admin_email = lookup("LEMONTODO_ADMIN_EMAIL")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let admin_password = lookup("LEMONTODO_ADMIN_PASSWORD").filter(|value| !value.is_empty());

        Ok(Self {
            host,
            port,
            database_path,
            allow_registration,
            admin_email,
            admin_password,
        })
    }

    pub fn bind_addr(&self) -> Result<SocketAddr> {
        format!("{}:{}", self.host, self.port)
            .parse()
            .with_context(|| format!("invalid bind address {}:{}", self.host, self.port))
    }
}

#[derive(Clone)]
pub struct AppState {
    store: Arc<Mutex<ServerStore>>,
}

impl AppState {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            store: Arc::new(Mutex::new(ServerStore::open(path)?)),
        })
    }
}

pub struct ServerStore {
    conn: Connection,
}

impl ServerStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create server data directory {}",
                    parent.display()
                )
            })?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("failed to open server database {}", path.display()))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn push(&mut self, request: PushRequest) -> Result<PushResponse> {
        if request.protocol_version != PROTOCOL_VERSION {
            return Ok(PushResponse {
                protocol_version: PROTOCOL_VERSION,
                accepted: Vec::new(),
                rejected: request
                    .objects
                    .into_iter()
                    .map(|object| RejectedSyncObject {
                        operation_id: object.operation_id,
                        reason: RejectionReason::UnsupportedProtocol,
                    })
                    .collect(),
                cursor: self.current_cursor()?,
            });
        }

        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        let tx = self.conn.transaction()?;
        for object in request.objects {
            if has_operation(&tx, object.operation_id)? {
                rejected.push(RejectedSyncObject {
                    operation_id: object.operation_id,
                    reason: RejectionReason::Duplicate,
                });
                continue;
            }

            insert_sync_object(&tx, &object)?;
            accepted.push(AcceptedSyncObject {
                operation_id: object.operation_id,
                object_id: object.object_id,
                object_revision: object.object_revision,
            });
        }
        tx.commit()?;

        Ok(PushResponse {
            protocol_version: PROTOCOL_VERSION,
            accepted,
            rejected,
            cursor: self.current_cursor()?,
        })
    }

    fn current_cursor(&self) -> Result<String> {
        let seq = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(server_seq), 0) FROM sync_objects",
                [],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to read server cursor")?;
        Ok(seq.to_string())
    }

    #[cfg(test)]
    fn count_sync_objects(&self) -> Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM sync_objects", [], |row| {
                row.get::<_, usize>(0)
            })
            .context("failed to count sync objects")
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sync_objects (
                server_seq INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT NOT NULL,
                operation_id TEXT NOT NULL UNIQUE,
                device_id TEXT NOT NULL,
                object_id TEXT NOT NULL,
                object_revision INTEGER NOT NULL,
                object_type TEXT NOT NULL,
                operation_type TEXT NOT NULL,
                created_at TEXT NOT NULL,
                received_at TEXT NOT NULL,
                envelope TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_sync_objects_cursor
                ON sync_objects(server_seq);
            CREATE INDEX IF NOT EXISTS idx_sync_objects_object
                ON sync_objects(object_id, object_revision);
            ",
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/server-info", get(server_info))
        .route("/v1/sync/push", axum::routing::post(sync_push))
        .with_state(state)
}

pub async fn serve(config: ServerConfig) -> Result<()> {
    let state = AppState::open(&config.database_path)?;
    let listener = tokio::net::TcpListener::bind(config.bind_addr()?)
        .await
        .context("failed to bind LemonTodo server")?;
    axum::serve(listener, app(state))
        .await
        .context("LemonTodo server failed")
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn server_info() -> Json<ServerInfo> {
    Json(ServerInfo::minimal())
}

async fn sync_push(
    State(state): State<AppState>,
    Json(request): Json<PushRequest>,
) -> Result<Json<PushResponse>, (StatusCode, String)> {
    let mut store = state.store.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "store lock poisoned".to_owned(),
        )
    })?;
    store
        .push(request)
        .map(Json)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

fn parse_bool(value: &str, name: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("{name} must be one of true/false, 1/0, yes/no, on/off"),
    }
}

fn default_database_path() -> PathBuf {
    dirs::data_dir()
        .map(|path| path.join("lemontodo-server").join("server.db"))
        .unwrap_or_else(|| PathBuf::from(".lemontodo-server.db"))
}

fn has_operation(conn: &Connection, operation_id: Uuid) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sync_objects WHERE operation_id = ?1",
        params![operation_id.to_string()],
        |_| Ok(()),
    )
    .optional()
    .map(|value| value.is_some())
    .context("failed to check duplicate sync object")
}

fn insert_sync_object(conn: &Connection, object: &EncryptedSyncObject) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_objects (
            id, operation_id, device_id, object_id, object_revision,
            object_type, operation_type, created_at, received_at, envelope
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            object.id.to_string(),
            object.operation_id.to_string(),
            object.device_id.to_string(),
            object.object_id.to_string(),
            object.object_revision,
            object.object_type.as_str(),
            object.operation_type.as_str(),
            object.created_at.to_rfc3339(),
            Utc::now().to_rfc3339(),
            serde_json::to_string(&object.envelope)?,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use lemontodo_crypto::CryptoEnvelope;
    use lemontodo_sync::{PROTOCOL_VERSION, ServerCapability};

    use super::*;

    #[test]
    fn loads_default_config() {
        let config = ServerConfig::from_lookup(|_| None).unwrap();
        assert_eq!(config.host, DEFAULT_HOST);
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.database_path, default_database_path());
        assert!(!config.allow_registration);
        assert_eq!(config.admin_email, None);
        assert_eq!(config.admin_password, None);
    }

    #[test]
    fn loads_config_from_lookup() {
        let env = HashMap::from([
            ("LEMONTODO_SERVER_HOST", "0.0.0.0"),
            ("LEMONTODO_SERVER_PORT", "9000"),
            ("LEMONTODO_SERVER_DB", "/tmp/lemontodo-test.db"),
            ("LEMONTODO_ALLOW_REGISTRATION", "yes"),
            ("LEMONTODO_ADMIN_EMAIL", "admin@example.com"),
            ("LEMONTODO_ADMIN_PASSWORD", "dev-password"),
        ]);

        let config =
            ServerConfig::from_lookup(|key| env.get(key).map(ToString::to_string)).unwrap();
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 9000);
        assert_eq!(
            config.database_path,
            PathBuf::from("/tmp/lemontodo-test.db")
        );
        assert!(config.allow_registration);
        assert_eq!(config.admin_email, Some("admin@example.com".to_owned()));
        assert_eq!(config.admin_password, Some("dev-password".to_owned()));
    }

    #[test]
    fn rejects_invalid_bool_config() {
        let env = HashMap::from([("LEMONTODO_ALLOW_REGISTRATION", "maybe")]);
        let error = ServerConfig::from_lookup(|key| env.get(key).map(ToString::to_string))
            .unwrap_err()
            .to_string();
        assert!(error.contains("LEMONTODO_ALLOW_REGISTRATION"));
    }

    #[test]
    fn minimal_server_info_matches_protocol() {
        let info = ServerInfo::minimal();
        assert_eq!(info.protocol_version, PROTOCOL_VERSION);
        assert_eq!(
            info.capabilities,
            vec![ServerCapability::ObjectSync, ServerCapability::BatchPush]
        );
    }

    #[tokio::test]
    async fn health_handler_reports_ok() {
        let Json(response) = healthz().await;
        assert_eq!(response.status, "ok");
    }

    #[tokio::test]
    async fn server_info_handler_reports_protocol() {
        let Json(response) = server_info().await;
        assert_eq!(response.protocol_version, PROTOCOL_VERSION);
        assert_eq!(
            response.capabilities,
            vec![ServerCapability::ObjectSync, ServerCapability::BatchPush]
        );
    }

    #[test]
    fn stores_pushed_sync_objects_and_rejects_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(dir.path().join("server.db")).unwrap();
        let object = test_sync_object();

        let response = store
            .push(PushRequest {
                protocol_version: PROTOCOL_VERSION,
                device_id: object.device_id,
                base_cursor: None,
                objects: vec![object.clone()],
            })
            .unwrap();

        assert_eq!(response.accepted.len(), 1);
        assert!(response.rejected.is_empty());
        assert_eq!(response.cursor, "1");
        assert_eq!(store.count_sync_objects().unwrap(), 1);

        let response = store
            .push(PushRequest {
                protocol_version: PROTOCOL_VERSION,
                device_id: object.device_id,
                base_cursor: Some(response.cursor),
                objects: vec![object],
            })
            .unwrap();

        assert!(response.accepted.is_empty());
        assert_eq!(response.rejected.len(), 1);
        assert_eq!(response.rejected[0].reason, RejectionReason::Duplicate);
        assert_eq!(response.cursor, "1");
        assert_eq!(store.count_sync_objects().unwrap(), 1);
    }

    #[test]
    fn rejects_unsupported_push_protocol_without_storing() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(dir.path().join("server.db")).unwrap();
        let object = test_sync_object();

        let response = store
            .push(PushRequest {
                protocol_version: PROTOCOL_VERSION + 1,
                device_id: object.device_id,
                base_cursor: None,
                objects: vec![object],
            })
            .unwrap();

        assert!(response.accepted.is_empty());
        assert_eq!(response.rejected.len(), 1);
        assert_eq!(
            response.rejected[0].reason,
            RejectionReason::UnsupportedProtocol
        );
        assert_eq!(response.cursor, "0");
        assert_eq!(store.count_sync_objects().unwrap(), 0);
    }

    fn test_sync_object() -> EncryptedSyncObject {
        EncryptedSyncObject {
            id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            device_id: Uuid::new_v4(),
            object_id: Uuid::new_v4(),
            object_revision: 1,
            object_type: "task".to_owned(),
            operation_type: "create".to_owned(),
            created_at: Utc::now(),
            envelope: CryptoEnvelope {
                version: 1,
                cipher: "xchacha20poly1305".to_owned(),
                nonce: "nonce".to_owned(),
                ciphertext: "ciphertext".to_owned(),
            },
        }
    }
}
