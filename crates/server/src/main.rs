use anyhow::Result;
use lemontodo_server::{ServerConfig, serve};

#[tokio::main]
async fn main() -> Result<()> {
    let config = ServerConfig::from_env()?;
    println!("Starting LemonTodo server on {}", config.bind_addr()?);
    serve(config).await
}
