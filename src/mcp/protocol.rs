use super::types::*;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// MCP Protocol handler for JSON-RPC over stdio
pub struct Protocol {
    reader: BufReader<tokio::io::Stdin>,
    writer: tokio::io::Stdout,
}

impl Protocol {
    pub fn new() -> Self {
        Self {
            reader: BufReader::new(tokio::io::stdin()),
            writer: tokio::io::stdout(),
        }
    }

    /// Read next JSON-RPC request from stdin
    pub async fn read_request(&mut self) -> Result<Option<JsonRpcRequest>> {
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line).await {
                Ok(0) => return Ok(None), // EOF
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue; // Skip empty lines
                    }
                    let request: JsonRpcRequest = serde_json::from_str(trimmed)?;
                    return Ok(Some(request));
                }
                Err(_) => return Ok(None),
            }
        }
    }

    /// Send JSON-RPC response to stdout
    pub async fn send_response(&mut self, response: JsonRpcResponse) -> Result<()> {
        let json = serde_json::to_string(&response)?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Send notification (reserved for future use)
    #[allow(dead_code)]
    pub async fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = Notification {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
        };
        let json = serde_json::to_string(&notification)?;
        self.writer.write_all(json.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Create success response
    pub fn success_response<T: Serialize>(&self, id: Value, result: T) -> JsonRpcResponse<T> {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Create error response
    pub fn error_response(&self, id: Value, error: JsonRpcError) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}
