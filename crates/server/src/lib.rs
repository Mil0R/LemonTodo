use std::{env, net::SocketAddr};

use anyhow::{Context, Result};
use axum::{Json, Router, routing::get};
use lemontodo_sync::ServerInfo;
use serde::Serialize;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8787;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

pub fn app() -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/server-info", get(server_info))
}

pub async fn serve(config: ServerConfig) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(config.bind_addr()?)
        .await
        .context("failed to bind LemonTodo server")?;
    axum::serve(listener, app())
        .await
        .context("LemonTodo server failed")
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn server_info() -> Json<ServerInfo> {
    Json(ServerInfo::minimal())
}

fn parse_bool(value: &str, name: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("{name} must be one of true/false, 1/0, yes/no, on/off"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use lemontodo_sync::{PROTOCOL_VERSION, ServerCapability};

    use super::*;

    #[test]
    fn loads_default_config() {
        let config = ServerConfig::from_lookup(|_| None).unwrap();
        assert_eq!(config.host, DEFAULT_HOST);
        assert_eq!(config.port, DEFAULT_PORT);
        assert!(!config.allow_registration);
        assert_eq!(config.admin_email, None);
        assert_eq!(config.admin_password, None);
    }

    #[test]
    fn loads_config_from_lookup() {
        let env = HashMap::from([
            ("LEMONTODO_SERVER_HOST", "0.0.0.0"),
            ("LEMONTODO_SERVER_PORT", "9000"),
            ("LEMONTODO_ALLOW_REGISTRATION", "yes"),
            ("LEMONTODO_ADMIN_EMAIL", "admin@example.com"),
            ("LEMONTODO_ADMIN_PASSWORD", "dev-password"),
        ]);

        let config =
            ServerConfig::from_lookup(|key| env.get(key).map(ToString::to_string)).unwrap();
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 9000);
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
        assert_eq!(info.capabilities, vec![ServerCapability::ObjectSync]);
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
        assert_eq!(response.capabilities, vec![ServerCapability::ObjectSync]);
    }
}
