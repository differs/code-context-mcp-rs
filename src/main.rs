mod mcp;
mod embedding;
mod vector_db;
mod parser;
mod snapshot;
mod handlers;

use anyhow::Result;
use std::path::PathBuf;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use mcp::server::McpServer;

/// Load .env files from multiple locations with priority order:
/// 1. Current working directory (project-specific config)
/// 2. XDG config directory ~/.config/code-context-mcp/.env (global default config)
///
/// Environment variables set directly in the shell always take highest priority.
fn load_env_files() {
    // Try current working directory first (project-specific)
    let cwd_env = std::env::current_dir()
        .map(|p| p.join(".env"))
        .ok();
    if let Some(path) = cwd_env {
        if path.exists() {
            if let Ok(_) = dotenv::from_path(&path) {
                tracing::debug!("Loaded .env from: {}", path.display());
                return; // Found and loaded, don't continue
            }
        }
    }

    // Try XDG config directory (global default)
    if let Some(config_dir) = get_xdg_config_dir() {
        let xdg_env = config_dir.join("code-context-mcp").join(".env");
        if xdg_env.exists() {
            if let Ok(_) = dotenv::from_path(&xdg_env) {
                tracing::debug!("Loaded .env from: {}", xdg_env.display());
                return;
            }
        }
    }

    // No .env file found, that's okay - user may use environment variables directly
    tracing::debug!("No .env file found, using environment variables only");
}

/// Get XDG config directory, fallback to ~/.config
fn get_xdg_config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
        })
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables from .env files (multi-location support)
    load_env_files();

    // Initialize logging - send logs to stderr to avoid interfering with MCP protocol
    // Default to "error" level to minimize output in TUI environments
    // Users can override with RUST_LOG environment variable (e.g., RUST_LOG=debug)
    let env_filter = EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| EnvFilter::new("error"));

    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(env_filter)
        .init();

    tracing::info!("Starting Code Context MCP server...");

    // Create and start MCP server
    let server = McpServer::new()?;
    server.start().await?;

    Ok(())
}
