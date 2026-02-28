use super::protocol::Protocol;
use super::types::*;
use crate::embedding::ollama::OllamaEmbedding;
use crate::embedding::EmbeddingProvider;
use crate::handlers::tool_handlers::ToolHandlers;
use crate::snapshot::SnapshotManager;
use crate::vector_db::milvus::MilvusVectorDatabase;
use crate::vector_db::VectorDatabase;
use anyhow::Result;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "code-context-mcp";
const SERVER_VERSION: &str = "0.1.0";

/// Main MCP Server
pub struct McpServer {
    protocol: Protocol,
    #[allow(dead_code)] // Passed to tool_handlers, retained for potential future direct use
    embedding: Arc<dyn EmbeddingProvider>,
    #[allow(dead_code)] // Passed to tool_handlers, retained for potential future direct use
    vector_db: Arc<dyn VectorDatabase>,
    snapshot_manager: Arc<SnapshotManager>,
    tool_handlers: Arc<Mutex<ToolHandlers>>,
}

impl McpServer {
    pub fn new() -> Result<Self> {
        // Get configuration from environment
        let ollama_host = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
        let embedding_model = std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "nomic-embed-text".to_string());
        let milvus_address = std::env::var("MILVUS_ADDRESS").unwrap_or_else(|_| "http://127.0.0.1:19530".to_string());

        // Initialize embedding provider
        let embedding = Arc::new(OllamaEmbedding::new(&ollama_host, &embedding_model));

        // Initialize vector database
        let vector_db = Arc::new(MilvusVectorDatabase::new(&milvus_address));

        // Initialize snapshot manager
        let snapshot_path = std::env::var("SNAPSHOT_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                PathBuf::from(home).join(".code-context/snapshot.json")
            });

        let snapshot_manager = Arc::new(SnapshotManager::new(snapshot_path)?);

        // Initialize tool handlers
        let tool_handlers = Arc::new(Mutex::new(ToolHandlers::new(
            embedding.clone(),
            vector_db.clone(),
            snapshot_manager.clone(),
        )));

        Ok(Self {
            protocol: Protocol::new(),
            embedding,
            vector_db,
            snapshot_manager,
            tool_handlers,
        })
    }

    pub async fn start(mut self) -> Result<()> {
        // Load existing snapshot
        self.snapshot_manager.load().await?;

        tracing::info!("MCP server started, waiting for requests...");

        // Main request loop
        loop {
            match self.protocol.read_request().await {
                Ok(Some(request)) => {
                    let response = self.handle_request(request).await;
                    if let Err(e) = self.protocol.send_response(response).await {
                        tracing::error!("Failed to send response: {}", e);
                    }
                }
                Ok(None) => {
                    tracing::info!("Client disconnected");
                    break;
                }
                Err(e) => {
                    tracing::error!("Failed to read request: {}", e);
                    let error_response = self.protocol.error_response(
                        json!(null),
                        JsonRpcError::parse_error(),
                    );
                    let _ = self.protocol.send_response(error_response).await;
                }
            }
        }

        Ok(())
    }

    async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        tracing::debug!("Received request: method={}, id={:?}", request.method, request.id);

        match request.method.as_str() {
            "initialize" => self.handle_initialize(request.id, request.params).await,
            "notifications/initialized" => {
                // Notification - no response needed per MCP spec
                // Return empty response to avoid client waiting
                JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: Some(json!({})),
                    error: None,
                }
            }
            "tools/list" => self.handle_tools_list(request.id).await,
            "tools/call" => self.handle_tools_call(request.id, request.params).await,
            _ => {
                self.protocol.error_response(request.id, JsonRpcError::method_not_found())
            }
        }
    }

    async fn handle_initialize(&self, id: serde_json::Value, params: serde_json::Value) -> JsonRpcResponse {
        // Parse client initialize request to validate protocol
        let client_info: InitializeRequest = match serde_json::from_value::<InitializeRequest>(params.clone()) {
            Ok(req) => {
                tracing::info!("Client connected: {} v{}", req.clientInfo.name, req.clientInfo.version);
                req
            }
            Err(e) => {
                tracing::warn!("Failed to parse initialize request (client may use simplified format): {}", e);
                // Continue anyway - some clients send minimal initialize params
                return self.protocol.error_response(
                    id,
                    JsonRpcError::internal_error(format!("Invalid initialize params: {}", e)),
                );
            }
        };

        // Store client info for future use (roots capability, etc.)
        let _ = client_info;

        let response = InitializeResponse {
            protocolVersion: PROTOCOL_VERSION.to_string(),
            capabilities: ServerCapabilities {
                tools: ToolsCapability {
                    listChanged: Some(true),
                },
            },
            serverInfo: ServerInfo {
                name: SERVER_NAME.to_string(),
                version: SERVER_VERSION.to_string(),
            },
        };

        self.protocol.success_response(id, json!(response))
    }

    async fn handle_tools_list(&self, id: serde_json::Value) -> JsonRpcResponse {
        let tools = vec![
            Tool {
                name: "index_codebase".to_string(),
                description: r#"Index a codebase directory to enable semantic search.

⚠️ **IMPORTANT**:
- You MUST provide an absolute path to the target codebase.
- Relative paths will be automatically resolved to absolute paths.

✨ **Usage Guidance**:
- This tool is typically used when search fails due to an unindexed codebase.
- If indexing is attempted on an already indexed path, you MUST prompt the user to confirm whether to proceed with a force index."#.to_string(),
                inputSchema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "ABSOLUTE path to the codebase directory to index."
                        },
                        "force": {
                            "type": "boolean",
                            "description": "Force re-indexing even if already indexed",
                            "default": false
                        },
                        "splitter": {
                            "type": "string",
                            "description": "Code splitter to use: 'ast' or 'langchain'",
                            "enum": ["ast", "langchain"],
                            "default": "ast"
                        }
                    },
                    "required": ["path"]
                }),
            },
            Tool {
                name: "search_code".to_string(),
                description: r#"Search the indexed codebase using natural language queries.

⚠️ **IMPORTANT**:
- You MUST provide an absolute path.
- If the codebase is not indexed, this tool will return an error."#.to_string(),
                inputSchema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "ABSOLUTE path to the codebase directory to search in."
                        },
                        "query": {
                            "type": "string",
                            "description": "Natural language query to search for in the codebase"
                        },
                        "limit": {
                            "type": "number",
                            "description": "Maximum number of results to return",
                            "default": 10,
                            "maximum": 50
                        }
                    },
                    "required": ["path", "query"]
                }),
            },
            Tool {
                name: "clear_index".to_string(),
                description: "Clear the search index for a codebase.".to_string(),
                inputSchema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "ABSOLUTE path to the codebase directory to clear."
                        }
                    },
                    "required": ["path"]
                }),
            },
            Tool {
                name: "get_indexing_status".to_string(),
                description: "Get the current indexing status of a codebase.".to_string(),
                inputSchema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "ABSOLUTE path to the codebase directory."
                        }
                    },
                    "required": ["path"]
                }),
            },
        ];

        let response = ListToolsResponse { tools };
        self.protocol.success_response(id, json!(response))
    }

    async fn handle_tools_call(&self, id: serde_json::Value, params: serde_json::Value) -> JsonRpcResponse {
        let call_request: CallToolRequest = match serde_json::from_value(params) {
            Ok(req) => req,
            Err(e) => {
                return self.protocol.error_response(
                    id,
                    JsonRpcError::internal_error(format!("Invalid params: {}", e)),
                );
            }
        };

        let handlers = self.tool_handlers.lock().await;
        let result = match call_request.name.as_str() {
            "index_codebase" => handlers.handle_index_codebase(&call_request.arguments).await,
            "search_code" => handlers.handle_search_code(&call_request.arguments).await,
            "clear_index" => handlers.handle_clear_index(&call_request.arguments).await,
            "get_indexing_status" => handlers.handle_get_indexing_status(&call_request.arguments).await,
            _ => {
                return self.protocol.error_response(
                    id,
                    JsonRpcError::internal_error(format!("Unknown tool: {}", call_request.name)),
                );
            }
        };

        match result {
            Ok(content) => {
                let response = CallToolResponse {
                    content,
                    isError: None,
                };
                self.protocol.success_response(id, json!(response))
            }
            Err(e) => {
                let response = CallToolResponse {
                    content: vec![Content::Text {
                        text: format!("Error: {}", e),
                    }],
                    isError: Some(true),
                };
                self.protocol.success_response(id, json!(response))
            }
        }
    }
}
