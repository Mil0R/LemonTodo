use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::{DateTime, Duration, Utc};
use lemontodo_sync::{
    AcceptedSyncObject, AccountStatusResponse, EncryptedSyncObject, LoginRequest, LoginResponse,
    LogoutRequest, LogoutResponse, PROTOCOL_VERSION, PullRequest, PullResponse, PushRequest,
    PushResponse, PutVaultMetadataRequest, RegisterRequest, RegisterResponse, RejectedSyncObject,
    RejectionReason, ServerAuthInfo, ServerCapability, ServerInfo, SessionInfo, SessionsResponse,
    VaultMetadataResponse,
};
use rand::rngs::OsRng;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8787;
const DEFAULT_SESSION_TTL_SECS: i64 = 60 * 60 * 24 * 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub database_path: PathBuf,
    pub allow_registration: bool,
    pub session_ttl_secs: i64,
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
        let session_ttl_secs = lookup("LEMONTODO_SESSION_TTL_SECS")
            .map(|value| {
                value
                    .parse::<i64>()
                    .with_context(|| format!("invalid LEMONTODO_SESSION_TTL_SECS: {value}"))
            })
            .transpose()?
            .unwrap_or(DEFAULT_SESSION_TTL_SECS);
        if session_ttl_secs <= 0 {
            anyhow::bail!("LEMONTODO_SESSION_TTL_SECS must be greater than 0");
        }
        let admin_email = lookup("LEMONTODO_ADMIN_EMAIL")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let admin_password = lookup("LEMONTODO_ADMIN_PASSWORD").filter(|value| !value.is_empty());

        Ok(Self {
            host,
            port,
            database_path,
            allow_registration,
            session_ttl_secs,
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
    config: ServerConfig,
}

impl AppState {
    pub fn open(config: ServerConfig) -> Result<Self> {
        let mut store = ServerStore::open(&config.database_path)?;
        store.ensure_admin_user(
            config.admin_email.as_deref(),
            config.admin_password.as_deref(),
        )?;
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            config,
        })
    }
}

pub struct ServerStore {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserAccount {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub encrypted_vault_key_json: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSession {
    pub token: String,
    pub user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
    pub device_id: Option<Uuid>,
    pub device_name: Option<String>,
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

    pub fn ensure_admin_user(
        &mut self,
        admin_email: Option<&str>,
        admin_password: Option<&str>,
    ) -> Result<Option<UserAccount>> {
        let Some(email) = admin_email.map(str::trim).filter(|email| !email.is_empty()) else {
            return Ok(None);
        };
        let Some(password) = admin_password.filter(|password| !password.is_empty()) else {
            return Ok(None);
        };
        if self.find_user_by_email(email)?.is_some() {
            return Ok(None);
        }
        self.create_user(email, password, true).map(Some)
    }

    pub fn create_user(
        &mut self,
        email: &str,
        password: &str,
        is_admin: bool,
    ) -> Result<UserAccount> {
        let email = normalize_email(email)?;
        let password = password.trim();
        if password.is_empty() {
            anyhow::bail!("password cannot be empty");
        }
        if self.find_user_by_email(&email)?.is_some() {
            anyhow::bail!("user already exists: {email}");
        }

        let user = UserAccount {
            id: Uuid::new_v4(),
            email,
            password_hash: hash_password(password)?,
            is_admin,
            encrypted_vault_key_json: None,
            created_at: Utc::now(),
        };
        self.conn.execute(
            "INSERT INTO users (id, email, password_hash, is_admin, encrypted_vault_key_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                user.id.to_string(),
                user.email,
                user.password_hash,
                user.is_admin,
                user.encrypted_vault_key_json,
                user.created_at.to_rfc3339(),
            ],
        )?;
        Ok(user)
    }

    pub fn create_session(
        &mut self,
        user: &UserAccount,
        device_id: Option<Uuid>,
        device_name: Option<&str>,
    ) -> Result<UserSession> {
        let session = UserSession {
            token: Uuid::new_v4().to_string(),
            user_id: user.id,
            created_at: Utc::now(),
            last_used_at: Utc::now(),
            device_id,
            device_name: device_name
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        };
        self.conn.execute(
            "INSERT INTO user_sessions (token, user_id, created_at, last_used_at, device_id, device_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session.token,
                session.user_id.to_string(),
                session.created_at.to_rfc3339(),
                session.last_used_at.to_rfc3339(),
                session.device_id.map(|value| value.to_string()),
                session.device_name,
            ],
        )?;
        Ok(session)
    }

    pub fn authenticate(
        &mut self,
        access_token: &str,
        session_ttl: Duration,
    ) -> Result<UserAccount> {
        let (user, _) = self.authenticate_with_session(access_token, session_ttl)?;
        Ok(user)
    }

    pub fn authenticate_with_session(
        &mut self,
        access_token: &str,
        session_ttl: Duration,
    ) -> Result<(UserAccount, UserSession)> {
        let mut session = self
            .find_session(access_token)?
            .with_context(|| "invalid access token".to_owned())?;
        if session.last_used_at + session_ttl < Utc::now() {
            self.conn
                .execute(
                    "DELETE FROM user_sessions WHERE token = ?1",
                    params![session.token],
                )
                .context("failed to delete expired session")?;
            anyhow::bail!("expired access token");
        }
        session.last_used_at = Utc::now();
        self.conn
            .execute(
                "UPDATE user_sessions SET last_used_at = ?1 WHERE token = ?2",
                params![session.last_used_at.to_rfc3339(), session.token],
            )
            .context("failed to update session activity")?;
        let user = self
            .find_user_by_id(session.user_id)?
            .with_context(|| format!("session user does not exist: {}", session.user_id))?;
        Ok((user, session))
    }

    pub fn revoke_session(&mut self, access_token: &str) -> Result<bool> {
        let token = access_token.trim();
        if token.is_empty() {
            anyhow::bail!("access token cannot be empty");
        }
        let deleted = self
            .conn
            .execute("DELETE FROM user_sessions WHERE token = ?1", params![token])
            .context("failed to revoke session")?;
        Ok(deleted > 0)
    }

    pub fn active_sessions(
        &mut self,
        user_id: Uuid,
        current_token: &str,
        session_ttl: Duration,
    ) -> Result<Vec<SessionInfo>> {
        let cutoff = Utc::now() - session_ttl;
        self.conn
            .execute(
                "DELETE FROM user_sessions WHERE user_id = ?1 AND last_used_at < ?2",
                params![user_id.to_string(), cutoff.to_rfc3339()],
            )
            .context("failed to delete expired sessions")?;

        let mut stmt = self
            .conn
            .prepare(
                "SELECT token, user_id, created_at, last_used_at, device_id, device_name
                 FROM user_sessions
                 WHERE user_id = ?1
                 ORDER BY last_used_at DESC",
            )
            .context("failed to prepare session query")?;
        let sessions = stmt
            .query_map(params![user_id.to_string()], row_to_user_session)?
            .map(|row| {
                row.map(|session| SessionInfo {
                    session_id: token_prefix(&session.token),
                    current: session.token == current_token,
                    created_at: session.created_at,
                    last_used_at: session.last_used_at,
                    device_id: session.device_id,
                    device_name: session.device_name,
                })
            })
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to query sessions")?;
        Ok(sessions)
    }

    pub fn set_encrypted_vault_key(
        &mut self,
        user_id: Uuid,
        encrypted_vault_key_json: &str,
    ) -> Result<()> {
        self.conn
            .execute(
                "UPDATE users SET encrypted_vault_key_json = ?1 WHERE id = ?2",
                params![encrypted_vault_key_json, user_id.to_string()],
            )
            .context("failed to store encrypted vault key")?;
        Ok(())
    }

    pub fn push(&mut self, user_id: Uuid, request: PushRequest) -> Result<PushResponse> {
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
                cursor: self.current_cursor(user_id)?,
            });
        }

        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        let tx = self.conn.transaction()?;
        for object in request.objects {
            if has_operation(&tx, user_id, object.operation_id)? {
                rejected.push(RejectedSyncObject {
                    operation_id: object.operation_id,
                    reason: RejectionReason::Duplicate,
                });
                continue;
            }

            insert_sync_object(&tx, user_id, &object)?;
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
            cursor: self.current_cursor(user_id)?,
        })
    }

    pub fn pull(&self, user_id: Uuid, request: PullRequest) -> Result<PullResponse> {
        let current_cursor = self.current_cursor(user_id)?;
        if request.protocol_version != PROTOCOL_VERSION {
            return Ok(PullResponse {
                protocol_version: PROTOCOL_VERSION,
                cursor: current_cursor,
                has_more: false,
                objects: Vec::new(),
            });
        }

        let cursor = parse_cursor(request.cursor.as_deref())?;
        let limit = normalized_limit(request.limit);
        let mut stmt = self.conn.prepare(
            "SELECT
                server_seq, user_id, id, operation_id, device_id, object_id, object_revision,
                object_type, operation_type, created_at, envelope
             FROM sync_objects
             WHERE user_id = ?1 AND server_seq > ?2
             ORDER BY server_seq ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![user_id.to_string(), cursor, limit + 1],
            row_to_sync_object,
        )?;
        let mut rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let has_more = rows.len() > limit as usize;
        if has_more {
            rows.truncate(limit as usize);
        }

        let next_cursor = rows
            .last()
            .map(|row| row.server_seq.to_string())
            .unwrap_or_else(|| current_cursor.clone());

        Ok(PullResponse {
            protocol_version: PROTOCOL_VERSION,
            cursor: next_cursor,
            has_more,
            objects: rows.into_iter().map(|row| row.object).collect(),
        })
    }

    fn current_cursor(&self, user_id: Uuid) -> Result<String> {
        let seq = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(server_seq), 0) FROM sync_objects WHERE user_id = ?1",
                params![user_id.to_string()],
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

    #[cfg(test)]
    fn count_users(&self) -> Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM users", [], |row| {
                row.get::<_, usize>(0)
            })
            .context("failed to count users")
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                is_admin INTEGER NOT NULL DEFAULT 0,
                encrypted_vault_key_json TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sync_objects (
                server_seq INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id TEXT NOT NULL,
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

            CREATE TABLE IF NOT EXISTS user_sessions (
                token TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                last_used_at TEXT NOT NULL,
                device_id TEXT,
                device_name TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_sync_objects_cursor
                ON sync_objects(server_seq);
            CREATE INDEX IF NOT EXISTS idx_sync_objects_object
                ON sync_objects(object_id, object_revision);
            CREATE INDEX IF NOT EXISTS idx_sync_objects_user_cursor
                ON sync_objects(user_id, server_seq);
            CREATE INDEX IF NOT EXISTS idx_users_email
                ON users(email);
            CREATE INDEX IF NOT EXISTS idx_user_sessions_user
                ON user_sessions(user_id);
            ",
        )?;
        add_column_if_missing(
            &self.conn,
            "sync_objects",
            "user_id",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        add_column_if_missing(&self.conn, "users", "encrypted_vault_key_json", "TEXT")?;
        add_column_if_missing(&self.conn, "user_sessions", "device_id", "TEXT")?;
        add_column_if_missing(&self.conn, "user_sessions", "device_name", "TEXT")?;
        Ok(())
    }

    fn find_user_by_id(&self, id: Uuid) -> Result<Option<UserAccount>> {
        self.conn
            .query_row(
                "SELECT id, email, password_hash, is_admin, encrypted_vault_key_json, created_at
                 FROM users WHERE id = ?1",
                params![id.to_string()],
                row_to_user_account,
            )
            .optional()
            .context("failed to query user by id")
    }

    fn find_user_by_email(&self, email: &str) -> Result<Option<UserAccount>> {
        let email = normalize_email(email)?;
        self.conn
            .query_row(
                "SELECT id, email, password_hash, is_admin, encrypted_vault_key_json, created_at
                 FROM users WHERE email = ?1",
                params![email],
                row_to_user_account,
            )
            .optional()
            .context("failed to query user by email")
    }

    fn find_session(&self, access_token: &str) -> Result<Option<UserSession>> {
        let token = access_token.trim();
        if token.is_empty() {
            anyhow::bail!("access token cannot be empty");
        }
        self.conn
            .query_row(
                "SELECT token, user_id, created_at, last_used_at, device_id, device_name
                 FROM user_sessions WHERE token = ?1",
                params![token],
                row_to_user_session,
            )
            .optional()
            .context("failed to query session by token")
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
        .route(
            "/v1/account/register",
            axum::routing::post(account_register),
        )
        .route("/v1/account/login", axum::routing::post(account_login))
        .route("/v1/account/logout", axum::routing::post(account_logout))
        .route("/v1/account/me", get(account_me))
        .route("/v1/account/sessions", get(account_sessions))
        .route("/v1/account/vault-key", get(account_get_vault_key))
        .route(
            "/v1/account/vault-key",
            axum::routing::put(account_put_vault_key),
        )
        .route("/v1/sync/push", axum::routing::post(sync_push))
        .route("/v1/sync/pull", axum::routing::post(sync_pull))
        .with_state(state)
}

pub async fn serve(config: ServerConfig) -> Result<()> {
    let state = AppState::open(config.clone())?;
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

async fn server_info(State(state): State<AppState>) -> Json<ServerInfo> {
    let mut info = ServerInfo::minimal();
    info.auth = ServerAuthInfo {
        password_auth: true,
        registration_allowed: state.config.allow_registration,
    };
    if state.config.allow_registration
        && !info
            .capabilities
            .contains(&ServerCapability::AccountRegistration)
    {
        info.capabilities
            .push(ServerCapability::AccountRegistration);
    }
    Json(info)
}

async fn account_register(
    State(state): State<AppState>,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, (StatusCode, String)> {
    if !state.config.allow_registration {
        return Err((StatusCode::FORBIDDEN, "registration is disabled".to_owned()));
    }
    let mut store = state.store.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "store lock poisoned".to_owned(),
        )
    })?;
    let user = store
        .create_user(&request.email, &request.password, false)
        .map_err(|error| {
            let message = error.to_string();
            let status =
                if message.contains("already exists") || message.contains("cannot be empty") {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
            (status, message)
        })?;
    Ok(Json(RegisterResponse {
        user_id: user.id,
        email: user.email,
        is_admin: user.is_admin,
    }))
}

async fn account_login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    let mut store = state.store.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "store lock poisoned".to_owned(),
        )
    })?;
    let user = store
        .find_user_by_email(&request.email)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .ok_or((StatusCode::UNAUTHORIZED, "invalid credentials".to_owned()))?;
    verify_password(&request.password, &user.password_hash)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid credentials".to_owned()))?;
    let session = store
        .create_session(&user, Some(request.device_id), Some(&request.device_name))
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(Json(LoginResponse {
        user_id: user.id,
        email: user.email,
        access_token: session.token,
    }))
}

async fn account_logout(
    State(state): State<AppState>,
    Json(request): Json<LogoutRequest>,
) -> Result<Json<LogoutResponse>, (StatusCode, String)> {
    let mut store = state.store.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "store lock poisoned".to_owned(),
        )
    })?;
    let revoked = store
        .revoke_session(&request.access_token)
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
    Ok(Json(LogoutResponse { revoked }))
}

async fn account_me(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<AccountStatusResponse>, (StatusCode, String)> {
    let access_token = query
        .get("access_token")
        .map(String::as_str)
        .ok_or((StatusCode::BAD_REQUEST, "missing access_token".to_owned()))?;
    let mut store = state.store.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "store lock poisoned".to_owned(),
        )
    })?;
    let (user, session) = store
        .authenticate_with_session(access_token, session_ttl(&state.config))
        .map_err(|error| (StatusCode::UNAUTHORIZED, error.to_string()))?;
    Ok(Json(AccountStatusResponse {
        user_id: user.id,
        email: user.email,
        is_admin: user.is_admin,
        has_vault_key: user.encrypted_vault_key_json.is_some(),
        user_created_at: user.created_at,
        session_created_at: session.created_at,
        session_last_used_at: session.last_used_at,
        session_device_id: session.device_id,
        session_device_name: session.device_name,
    }))
}

async fn account_sessions(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<SessionsResponse>, (StatusCode, String)> {
    let access_token = query
        .get("access_token")
        .map(String::as_str)
        .ok_or((StatusCode::BAD_REQUEST, "missing access_token".to_owned()))?;
    let mut store = state.store.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "store lock poisoned".to_owned(),
        )
    })?;
    let (user, _) = store
        .authenticate_with_session(access_token, session_ttl(&state.config))
        .map_err(|error| (StatusCode::UNAUTHORIZED, error.to_string()))?;
    let sessions = store
        .active_sessions(user.id, access_token, session_ttl(&state.config))
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(Json(SessionsResponse { sessions }))
}

async fn account_get_vault_key(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<VaultMetadataResponse>, (StatusCode, String)> {
    let access_token = query
        .get("access_token")
        .map(String::as_str)
        .ok_or((StatusCode::BAD_REQUEST, "missing access_token".to_owned()))?;
    let mut store = state.store.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "store lock poisoned".to_owned(),
        )
    })?;
    let (user, _) = store
        .authenticate_with_session(access_token, session_ttl(&state.config))
        .map_err(|error| (StatusCode::UNAUTHORIZED, error.to_string()))?;
    let encrypted_vault_key = user
        .encrypted_vault_key_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to parse stored vault metadata: {error}"),
            )
        })?;
    Ok(Json(VaultMetadataResponse {
        has_vault_key: encrypted_vault_key.is_some(),
        encrypted_vault_key,
    }))
}

async fn account_put_vault_key(
    State(state): State<AppState>,
    Json(request): Json<PutVaultMetadataRequest>,
) -> Result<Json<VaultMetadataResponse>, (StatusCode, String)> {
    let mut store = state.store.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "store lock poisoned".to_owned(),
        )
    })?;
    let (user, _) = store
        .authenticate_with_session(&request.access_token, session_ttl(&state.config))
        .map_err(|error| (StatusCode::UNAUTHORIZED, error.to_string()))?;
    let json = serde_json::to_string(&request.encrypted_vault_key).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize vault metadata: {error}"),
        )
    })?;
    store
        .set_encrypted_vault_key(user.id, &json)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(Json(VaultMetadataResponse {
        has_vault_key: true,
        encrypted_vault_key: Some(request.encrypted_vault_key),
    }))
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
    let (user, _) = store
        .authenticate_with_session(&request.access_token, session_ttl(&state.config))
        .map_err(|error| (StatusCode::UNAUTHORIZED, error.to_string()))?;
    store
        .push(user.id, request)
        .map(Json)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

async fn sync_pull(
    State(state): State<AppState>,
    Json(request): Json<PullRequest>,
) -> Result<Json<PullResponse>, (StatusCode, String)> {
    let mut store = state.store.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "store lock poisoned".to_owned(),
        )
    })?;
    let (user, _) = store
        .authenticate_with_session(&request.access_token, session_ttl(&state.config))
        .map_err(|error| (StatusCode::UNAUTHORIZED, error.to_string()))?;
    store
        .pull(user.id, request)
        .map(Json)
        .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))
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

fn session_ttl(config: &ServerConfig) -> Duration {
    Duration::seconds(config.session_ttl_secs)
}

fn has_operation(conn: &Connection, user_id: Uuid, operation_id: Uuid) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sync_objects WHERE user_id = ?1 AND operation_id = ?2",
        params![user_id.to_string(), operation_id.to_string()],
        |_| Ok(()),
    )
    .optional()
    .map(|value| value.is_some())
    .context("failed to check duplicate sync object")
}

fn insert_sync_object(
    conn: &Connection,
    user_id: Uuid,
    object: &EncryptedSyncObject,
) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_objects (
            user_id, id, operation_id, device_id, object_id, object_revision,
            object_type, operation_type, created_at, received_at, envelope
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            user_id.to_string(),
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

struct StoredSyncObject {
    server_seq: i64,
    object: EncryptedSyncObject,
}

fn row_to_sync_object(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSyncObject> {
    let envelope = row.get::<_, String>(10)?;
    Ok(StoredSyncObject {
        server_seq: row.get(0)?,
        object: EncryptedSyncObject {
            id: parse_uuid(row.get::<_, String>(2)?)?,
            operation_id: parse_uuid(row.get::<_, String>(3)?)?,
            device_id: parse_uuid(row.get::<_, String>(4)?)?,
            object_id: parse_uuid(row.get::<_, String>(5)?)?,
            object_revision: row.get(6)?,
            object_type: row.get(7)?,
            operation_type: row.get(8)?,
            created_at: parse_datetime(row.get::<_, String>(9)?)?,
            envelope: serde_json::from_str(&envelope).map_err(to_sql_error)?,
        },
    })
}

fn parse_cursor(cursor: Option<&str>) -> Result<i64> {
    match cursor {
        Some(cursor) if !cursor.trim().is_empty() => cursor
            .parse::<i64>()
            .with_context(|| format!("invalid sync cursor: {cursor}")),
        _ => Ok(0),
    }
}

fn normalized_limit(limit: u32) -> i64 {
    let limit = if limit == 0 { 1 } else { limit };
    i64::from(limit.min(500))
}

fn token_prefix(token: &str) -> String {
    token.chars().take(8).collect()
}

fn normalize_email(email: &str) -> Result<String> {
    let email = email.trim().to_ascii_lowercase();
    if email.is_empty() {
        anyhow::bail!("email cannot be empty");
    }
    if !email.contains('@') {
        anyhow::bail!("email must contain @");
    }
    Ok(email)
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| anyhow::anyhow!("failed to hash password: {error}"))
}

fn verify_password(password: &str, password_hash: &str) -> Result<()> {
    let parsed = PasswordHash::new(password_hash)
        .map_err(|error| anyhow::anyhow!("failed to parse password hash: {error}"))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| anyhow::anyhow!("invalid credentials"))
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|existing| existing == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

fn parse_uuid(value: String) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(&value).map_err(to_sql_error)
}

fn parse_datetime(value: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|date| date.with_timezone(&Utc))
        .map_err(to_sql_error)
}

fn to_sql_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

fn row_to_user_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserAccount> {
    Ok(UserAccount {
        id: parse_uuid(row.get::<_, String>(0)?)?,
        email: row.get(1)?,
        password_hash: row.get(2)?,
        is_admin: row.get(3)?,
        encrypted_vault_key_json: row.get(4)?,
        created_at: parse_datetime(row.get::<_, String>(5)?)?,
    })
}

fn row_to_user_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserSession> {
    Ok(UserSession {
        token: row.get(0)?,
        user_id: parse_uuid(row.get::<_, String>(1)?)?,
        created_at: parse_datetime(row.get::<_, String>(2)?)?,
        last_used_at: parse_datetime(row.get::<_, String>(3)?)?,
        device_id: row
            .get::<_, Option<String>>(4)?
            .map(parse_uuid)
            .transpose()?,
        device_name: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, thread, time::Duration as StdDuration};

    use lemontodo_crypto::{CryptoEnvelope, KdfParams, VaultKey, wrap_vault_key};
    use lemontodo_sync::{
        LoginRequest, LogoutRequest, PROTOCOL_VERSION, PutVaultMetadataRequest, RegisterRequest,
        ServerCapability,
    };

    use super::*;

    #[test]
    fn loads_default_config() {
        let config = ServerConfig::from_lookup(|_| None).unwrap();
        assert_eq!(config.host, DEFAULT_HOST);
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.database_path, default_database_path());
        assert!(!config.allow_registration);
        assert_eq!(config.session_ttl_secs, DEFAULT_SESSION_TTL_SECS);
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
            ("LEMONTODO_SESSION_TTL_SECS", "3600"),
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
        assert_eq!(config.session_ttl_secs, 3600);
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
    fn rejects_invalid_session_ttl_config() {
        let env = HashMap::from([("LEMONTODO_SESSION_TTL_SECS", "0")]);
        let error = ServerConfig::from_lookup(|key| env.get(key).map(ToString::to_string))
            .unwrap_err()
            .to_string();
        assert!(error.contains("LEMONTODO_SESSION_TTL_SECS must be greater than 0"));
    }

    #[test]
    fn minimal_server_info_matches_protocol() {
        let info = ServerInfo::minimal();
        assert_eq!(info.protocol_version, PROTOCOL_VERSION);
        assert_eq!(
            info.capabilities,
            vec![
                ServerCapability::ObjectSync,
                ServerCapability::BatchPush,
                ServerCapability::CursorPull,
                ServerCapability::PasswordAuth,
            ]
        );
        assert!(info.auth.password_auth);
        assert!(!info.auth.registration_allowed);
    }

    #[tokio::test]
    async fn health_handler_reports_ok() {
        let Json(response) = healthz().await;
        assert_eq!(response.status, "ok");
    }

    #[tokio::test]
    async fn server_info_handler_reports_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(test_server_config(dir.path().join("server.db"))).unwrap();
        let Json(response) = server_info(State(state)).await;
        assert_eq!(response.protocol_version, PROTOCOL_VERSION);
        assert_eq!(
            response.capabilities,
            vec![
                ServerCapability::ObjectSync,
                ServerCapability::BatchPush,
                ServerCapability::CursorPull,
                ServerCapability::PasswordAuth,
            ]
        );
        assert!(response.auth.password_auth);
        assert!(!response.auth.registration_allowed);
    }

    #[test]
    fn bootstraps_admin_user_once() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("server.db");
        let mut store = ServerStore::open(&db_path).unwrap();
        let created = store
            .ensure_admin_user(Some("admin@example.com"), Some("dev-password"))
            .unwrap();
        assert!(created.is_some());
        assert_eq!(store.count_users().unwrap(), 1);

        let created = store
            .ensure_admin_user(Some("admin@example.com"), Some("dev-password"))
            .unwrap();
        assert!(created.is_none());
        assert_eq!(store.count_users().unwrap(), 1);
    }

    #[tokio::test]
    async fn register_handler_creates_user_when_registration_is_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_server_config(dir.path().join("server.db"));
        config.allow_registration = true;
        let state = AppState::open(config).unwrap();

        let Json(response) = account_register(
            State(state.clone()),
            Json(RegisterRequest {
                email: "User@example.com".to_owned(),
                password: "dev-password".to_owned(),
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.email, "user@example.com");
        assert!(!response.is_admin);
        let store = state.store.lock().unwrap();
        assert_eq!(store.count_users().unwrap(), 1);
    }

    #[tokio::test]
    async fn register_handler_rejects_when_registration_is_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::open(test_server_config(dir.path().join("server.db"))).unwrap();
        let error = account_register(
            State(state),
            Json(RegisterRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn login_handler_creates_session_with_valid_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_server_config(dir.path().join("server.db"));
        config.allow_registration = true;
        let state = AppState::open(config).unwrap();
        let _ = account_register(
            State(state.clone()),
            Json(RegisterRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
            }),
        )
        .await
        .unwrap();

        let Json(response) = account_login(
            State(state),
            Json(LoginRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
                device_id: Uuid::new_v4(),
                device_name: "test-device".to_owned(),
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.email, "user@example.com");
        assert!(!response.access_token.is_empty());
    }

    #[tokio::test]
    async fn logout_handler_revokes_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_server_config(dir.path().join("server.db"));
        config.allow_registration = true;
        let state = AppState::open(config).unwrap();
        let _ = account_register(
            State(state.clone()),
            Json(RegisterRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
            }),
        )
        .await
        .unwrap();
        let Json(login) = account_login(
            State(state.clone()),
            Json(LoginRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
                device_id: Uuid::new_v4(),
                device_name: "test-device".to_owned(),
            }),
        )
        .await
        .unwrap();

        let Json(response) = account_logout(
            State(state.clone()),
            Json(LogoutRequest {
                access_token: login.access_token.clone(),
            }),
        )
        .await
        .unwrap();

        assert!(response.revoked);
        let mut store = state.store.lock().unwrap();
        assert!(
            store
                .authenticate(&login.access_token, session_ttl(&state.config))
                .is_err()
        );
    }

    #[tokio::test]
    async fn vault_metadata_round_trip_is_user_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_server_config(dir.path().join("server.db"));
        config.allow_registration = true;
        let state = AppState::open(config).unwrap();
        let _ = account_register(
            State(state.clone()),
            Json(RegisterRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
            }),
        )
        .await
        .unwrap();
        let Json(login) = account_login(
            State(state.clone()),
            Json(LoginRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
                device_id: Uuid::new_v4(),
                device_name: "test-device".to_owned(),
            }),
        )
        .await
        .unwrap();
        let encrypted_vault_key = wrap_vault_key(
            &VaultKey::generate(),
            "master-password",
            KdfParams::generate_interactive(),
        )
        .unwrap();

        let Json(uploaded) = account_put_vault_key(
            State(state.clone()),
            Json(PutVaultMetadataRequest {
                access_token: login.access_token.clone(),
                encrypted_vault_key: encrypted_vault_key.clone(),
            }),
        )
        .await
        .unwrap();
        assert!(uploaded.has_vault_key);

        let mut query = std::collections::HashMap::new();
        query.insert("access_token".to_owned(), login.access_token.clone());
        let Json(downloaded) = account_get_vault_key(State(state), axum::extract::Query(query))
            .await
            .unwrap();
        assert!(downloaded.has_vault_key);
        assert_eq!(downloaded.encrypted_vault_key, Some(encrypted_vault_key));
    }

    #[tokio::test]
    async fn account_me_reports_user_and_updates_session_activity() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_server_config(dir.path().join("server.db"));
        config.allow_registration = true;
        let state = AppState::open(config).unwrap();
        let _ = account_register(
            State(state.clone()),
            Json(RegisterRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
            }),
        )
        .await
        .unwrap();
        let device_id = Uuid::new_v4();
        let Json(login) = account_login(
            State(state.clone()),
            Json(LoginRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
                device_id,
                device_name: "workstation".to_owned(),
            }),
        )
        .await
        .unwrap();

        let initial_session = {
            let store = state.store.lock().unwrap();
            store.find_session(&login.access_token).unwrap().unwrap()
        };
        thread::sleep(StdDuration::from_millis(10));

        let mut query = HashMap::new();
        query.insert("access_token".to_owned(), login.access_token.clone());
        let Json(status) = account_me(State(state.clone()), axum::extract::Query(query))
            .await
            .unwrap();

        assert_eq!(status.email, "user@example.com");
        assert!(!status.is_admin);
        assert!(!status.has_vault_key);
        assert_eq!(status.session_device_id, Some(device_id));
        assert_eq!(status.session_device_name.as_deref(), Some("workstation"));
        assert_eq!(status.session_created_at, initial_session.created_at);
        assert!(status.session_last_used_at > initial_session.last_used_at);

        let refreshed_session = {
            let store = state.store.lock().unwrap();
            store.find_session(&login.access_token).unwrap().unwrap()
        };
        assert_eq!(refreshed_session.last_used_at, status.session_last_used_at);
    }

    #[tokio::test]
    async fn account_me_rejects_expired_session_and_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_server_config(dir.path().join("server.db"));
        config.allow_registration = true;
        config.session_ttl_secs = 1;
        let state = AppState::open(config).unwrap();
        let _ = account_register(
            State(state.clone()),
            Json(RegisterRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
            }),
        )
        .await
        .unwrap();
        let Json(login) = account_login(
            State(state.clone()),
            Json(LoginRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
                device_id: Uuid::new_v4(),
                device_name: "test-device".to_owned(),
            }),
        )
        .await
        .unwrap();

        {
            let store = state.store.lock().unwrap();
            let expired_at = (Utc::now() - chrono::Duration::seconds(5)).to_rfc3339();
            store
                .conn
                .execute(
                    "UPDATE user_sessions SET last_used_at = ?1 WHERE token = ?2",
                    params![expired_at, login.access_token],
                )
                .unwrap();
        }

        let mut query = HashMap::new();
        query.insert("access_token".to_owned(), login.access_token.clone());
        let error = account_me(State(state.clone()), axum::extract::Query(query))
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
        assert_eq!(error.1, "expired access token");

        let store = state.store.lock().unwrap();
        assert!(store.find_session(&login.access_token).unwrap().is_none());
    }

    #[tokio::test]
    async fn account_sessions_lists_active_sessions_for_user() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_server_config(dir.path().join("server.db"));
        config.allow_registration = true;
        let state = AppState::open(config).unwrap();
        let _ = account_register(
            State(state.clone()),
            Json(RegisterRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
            }),
        )
        .await
        .unwrap();
        let first_device_id = Uuid::new_v4();
        let Json(first) = account_login(
            State(state.clone()),
            Json(LoginRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
                device_id: first_device_id,
                device_name: "laptop".to_owned(),
            }),
        )
        .await
        .unwrap();
        let second_device_id = Uuid::new_v4();
        let Json(second) = account_login(
            State(state.clone()),
            Json(LoginRequest {
                email: "user@example.com".to_owned(),
                password: "dev-password".to_owned(),
                device_id: second_device_id,
                device_name: "desktop".to_owned(),
            }),
        )
        .await
        .unwrap();

        let mut query = HashMap::new();
        query.insert("access_token".to_owned(), second.access_token.clone());
        let Json(response) = account_sessions(State(state), axum::extract::Query(query))
            .await
            .unwrap();

        assert_eq!(response.sessions.len(), 2);
        let current = response
            .sessions
            .iter()
            .find(|session| session.current)
            .unwrap();
        assert_eq!(current.session_id, token_prefix(&second.access_token));
        assert_eq!(current.device_id, Some(second_device_id));
        assert_eq!(current.device_name.as_deref(), Some("desktop"));
        let other = response
            .sessions
            .iter()
            .find(|session| session.session_id == token_prefix(&first.access_token))
            .unwrap();
        assert!(!other.current);
        assert_eq!(other.device_id, Some(first_device_id));
        assert_eq!(other.device_name.as_deref(), Some("laptop"));
    }

    #[test]
    fn stores_pushed_sync_objects_and_rejects_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(dir.path().join("server.db")).unwrap();
        let (user, _session) = create_test_user_and_session(&mut store);
        let object = test_sync_object();

        let response = store
            .push(
                user.id,
                PushRequest {
                    protocol_version: PROTOCOL_VERSION,
                    device_id: object.device_id,
                    access_token: "unused".to_owned(),
                    base_cursor: None,
                    objects: vec![object.clone()],
                },
            )
            .unwrap();

        assert_eq!(response.accepted.len(), 1);
        assert!(response.rejected.is_empty());
        assert_eq!(response.cursor, "1");
        assert_eq!(store.count_sync_objects().unwrap(), 1);

        let response = store
            .push(
                user.id,
                PushRequest {
                    protocol_version: PROTOCOL_VERSION,
                    device_id: object.device_id,
                    access_token: "unused".to_owned(),
                    base_cursor: Some(response.cursor),
                    objects: vec![object],
                },
            )
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
        let (user, _session) = create_test_user_and_session(&mut store);
        let object = test_sync_object();

        let response = store
            .push(
                user.id,
                PushRequest {
                    protocol_version: PROTOCOL_VERSION + 1,
                    device_id: object.device_id,
                    access_token: "unused".to_owned(),
                    base_cursor: None,
                    objects: vec![object],
                },
            )
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

    #[test]
    fn pulls_sync_objects_after_cursor_with_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(dir.path().join("server.db")).unwrap();
        let (user, _session) = create_test_user_and_session(&mut store);
        let first = test_sync_object();
        let second = test_sync_object();

        store
            .push(
                user.id,
                PushRequest {
                    protocol_version: PROTOCOL_VERSION,
                    device_id: first.device_id,
                    access_token: "unused".to_owned(),
                    base_cursor: None,
                    objects: vec![first.clone(), second.clone()],
                },
            )
            .unwrap();

        let response = store
            .pull(
                user.id,
                PullRequest {
                    protocol_version: PROTOCOL_VERSION,
                    device_id: Uuid::new_v4(),
                    access_token: "unused".to_owned(),
                    cursor: None,
                    limit: 1,
                },
            )
            .unwrap();

        assert_eq!(response.objects.len(), 1);
        assert_eq!(response.objects[0].operation_id, first.operation_id);
        assert_eq!(response.cursor, "1");
        assert!(response.has_more);

        let response = store
            .pull(
                user.id,
                PullRequest {
                    protocol_version: PROTOCOL_VERSION,
                    device_id: Uuid::new_v4(),
                    access_token: "unused".to_owned(),
                    cursor: Some(response.cursor),
                    limit: 10,
                },
            )
            .unwrap();

        assert_eq!(response.objects.len(), 1);
        assert_eq!(response.objects[0].operation_id, second.operation_id);
        assert_eq!(response.cursor, "2");
        assert!(!response.has_more);
    }

    #[test]
    fn unsupported_pull_protocol_returns_empty_current_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(dir.path().join("server.db")).unwrap();
        let (user, _session) = create_test_user_and_session(&mut store);
        let object = test_sync_object();
        store
            .push(
                user.id,
                PushRequest {
                    protocol_version: PROTOCOL_VERSION,
                    device_id: object.device_id,
                    access_token: "unused".to_owned(),
                    base_cursor: None,
                    objects: vec![object],
                },
            )
            .unwrap();

        let response = store
            .pull(
                user.id,
                PullRequest {
                    protocol_version: PROTOCOL_VERSION + 1,
                    device_id: Uuid::new_v4(),
                    access_token: "unused".to_owned(),
                    cursor: None,
                    limit: 10,
                },
            )
            .unwrap();

        assert!(response.objects.is_empty());
        assert_eq!(response.cursor, "1");
        assert!(!response.has_more);
    }

    #[test]
    fn pull_only_returns_objects_for_authenticated_user() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ServerStore::open(dir.path().join("server.db")).unwrap();
        let (user_a, _session_a) = create_test_user_and_session(&mut store);
        let (user_b, _session_b) =
            create_test_user_and_session_with_email(&mut store, "other@example.com");
        let first = test_sync_object();
        let second = test_sync_object();

        store
            .push(
                user_a.id,
                PushRequest {
                    protocol_version: PROTOCOL_VERSION,
                    device_id: first.device_id,
                    access_token: "unused".to_owned(),
                    base_cursor: None,
                    objects: vec![first.clone()],
                },
            )
            .unwrap();
        store
            .push(
                user_b.id,
                PushRequest {
                    protocol_version: PROTOCOL_VERSION,
                    device_id: second.device_id,
                    access_token: "unused".to_owned(),
                    base_cursor: None,
                    objects: vec![second.clone()],
                },
            )
            .unwrap();

        let response = store
            .pull(
                user_a.id,
                PullRequest {
                    protocol_version: PROTOCOL_VERSION,
                    device_id: Uuid::new_v4(),
                    access_token: "unused".to_owned(),
                    cursor: None,
                    limit: 10,
                },
            )
            .unwrap();

        assert_eq!(response.objects.len(), 1);
        assert_eq!(response.objects[0].operation_id, first.operation_id);
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

    fn test_server_config(database_path: PathBuf) -> ServerConfig {
        ServerConfig {
            host: DEFAULT_HOST.to_owned(),
            port: DEFAULT_PORT,
            database_path,
            allow_registration: false,
            session_ttl_secs: DEFAULT_SESSION_TTL_SECS,
            admin_email: None,
            admin_password: None,
        }
    }

    fn create_test_user_and_session(store: &mut ServerStore) -> (UserAccount, UserSession) {
        create_test_user_and_session_with_email(store, "user@example.com")
    }

    fn create_test_user_and_session_with_email(
        store: &mut ServerStore,
        email: &str,
    ) -> (UserAccount, UserSession) {
        let user = store.create_user(email, "dev-password", false).unwrap();
        let session = store
            .create_session(&user, Some(Uuid::new_v4()), Some("test-device"))
            .unwrap();
        (user, session)
    }
}
