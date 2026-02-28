use crate::embedding::EmbeddingProvider;
use crate::mcp::types::Content;
use crate::parser::code_parser::CodeParser;
use crate::snapshot::SnapshotManager;
use crate::vector_db::VectorDatabase;
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio::fs;

/// Tool handlers for MCP server
pub struct ToolHandlers {
    embedding: Arc<dyn EmbeddingProvider>,
    vector_db: Arc<dyn VectorDatabase>,
    snapshot_manager: Arc<SnapshotManager>,
    code_parser: CodeParser,
}

impl ToolHandlers {
    pub fn new(
        embedding: Arc<dyn EmbeddingProvider>,
        vector_db: Arc<dyn VectorDatabase>,
        snapshot_manager: Arc<SnapshotManager>,
    ) -> Self {
        Self {
            embedding,
            vector_db,
            snapshot_manager,
            code_parser: CodeParser::new(),
        }
    }

    /// Handle index_codebase tool
    pub async fn handle_index_codebase(&self, args: &Value) -> Result<Vec<Content>> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing 'path' argument")?;

        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
        let _splitter = args
            .get("splitter")
            .and_then(|v| v.as_str())
            .unwrap_or("ast");

        let path = Path::new(path);

        if !path.exists() {
            anyhow::bail!("Path does not exist: {}", path.display());
        }

        if !path.is_dir() {
            anyhow::bail!("Path is not a directory: {}", path.display());
        }

        // Generate collection name from path hash
        let path_hash = CodeParser::hash_file(&path.to_string_lossy());
        let collection_name = format!("code_index_{}", &path_hash[..16]);

        // Check if already indexed
        if let Some(existing_collection) = self.snapshot_manager.get_collection_name().await {
            if existing_collection == collection_name && !force {
                return Ok(vec![Content::Text {
                    text: format!(
                        "Codebase already indexed. Use force=true to re-index.\nCollection: {}",
                        collection_name
                    ),
                }]);
            }
        }

        // Create collection if not exists
        let dimension = self.embedding.dimension();
        
        // Try to create collection (may fail if already exists)
        let _ = self.vector_db.create_collection(&collection_name, dimension).await;

        tracing::info!("Indexing codebase at: {}", path.display());

        // Walk directory and index files
        let mut total_files = 0;
        let mut total_chunks = 0;

        let walker = WalkBuilder::new(path)
            .standard_filters(true)
            .hidden(true) // Skip hidden files
            .build();

        for entry in walker.flatten() {
            if entry.file_type().map_or(true, |ft| !ft.is_file()) {
                continue;
            }

            let file_path = entry.path();
            
            // Read file content
            let content = match fs::read_to_string(file_path).await {
                Ok(c) => c,
                Err(_) => continue, // Skip binary files
            };

            // Calculate hash
            let file_hash = CodeParser::hash_file(&content);

            // Check if file has changed
            if let Some(existing_hash) = self.snapshot_manager.get_file_hash(file_path).await {
                if existing_hash == file_hash {
                    continue; // Skip unchanged files
                }
            }

            // Parse and chunk code
            let chunks = match self.code_parser.parse(file_path, &content) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to parse {:?}: {}", file_path, e);
                    continue;
                }
            };

            if chunks.is_empty() {
                continue;
            }

            // Generate embeddings
            let texts: Vec<String> = chunks
                .iter()
                .map(|c| format!("{}\n{}", c.content, c.symbol_name.as_deref().unwrap_or("")))
                .collect();

            let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
            
            let embeddings = match self.embedding.embed_batch(&text_refs).await {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Failed to embed {:?}: {}", file_path, e);
                    continue;
                }
            };

            // Prepare metadata
            let metadata: Vec<Value> = chunks
                .iter()
                .map(|c| {
                    json!({
                        "file_path": c.file_path,
                        "start_line": c.start_line,
                        "end_line": c.end_line,
                        "symbol_name": c.symbol_name,
                        "symbol_kind": c.symbol_kind.as_str(),
                        "content": c.content,
                    })
                })
                .collect();

            let vectors: Vec<Vec<f32>> = embeddings.into_iter().map(|e| e.values).collect();

            // Insert into vector database
            if let Err(e) = self.vector_db.insert(&collection_name, &vectors, &metadata).await {
                tracing::warn!("Failed to insert vectors: {}", e);
                continue;
            }

            // Update snapshot
            self.snapshot_manager
                .update_file(file_path.to_path_buf(), file_hash, chunks.len())
                .await;

            total_files += 1;
            total_chunks += chunks.len();
        }

        // Save snapshot
        self.snapshot_manager.set_collection_name(collection_name.clone()).await;
        self.snapshot_manager.save().await?;

        Ok(vec![Content::Text {
            text: format!(
                "Indexed {} files, {} chunks\nCollection: {}",
                total_files, total_chunks, collection_name
            ),
        }])
    }

    /// Handle search_code tool
    pub async fn handle_search_code(&self, args: &Value) -> Result<Vec<Content>> {
        let _path = args
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing 'path' argument")?;

        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .context("Missing 'query' argument")?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let collection_name = self
            .snapshot_manager
            .get_collection_name()
            .await
            .context("No indexed codebase found. Please index first.")?;

        // Embed query
        let embedding = self.embedding.embed(query).await?;

        // Search vector database
        let results = self.vector_db.search(&collection_name, &embedding.values, limit).await?;

        if results.is_empty() {
            return Ok(vec![Content::Text {
                text: "No results found.".to_string(),
            }]);
        }

        // Format results
        let mut formatted = String::from("Search results:\n\n");
        for (i, result) in results.iter().enumerate() {
            let file_path = result
                .metadata
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let start_line = result
                .metadata
                .get("start_line")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let end_line = result
                .metadata
                .get("end_line")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let symbol_name = result
                .metadata
                .get("symbol_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let content = result
                .metadata
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            formatted.push_str(&format!(
                "{}. **{}** (`{}:{}-{}`)\nScore: {:.2}%\n```\n{}\n```\n\n",
                i + 1,
                symbol_name,
                file_path,
                start_line + 1,
                end_line + 1,
                result.score * 100.0,
                truncate(content, 500)
            ));
        }

        Ok(vec![Content::Text { text: formatted }])
    }

    /// Handle clear_index tool
    pub async fn handle_clear_index(&self, args: &Value) -> Result<Vec<Content>> {
        let __path = args
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing 'path' argument")?;

        let collection_name = self
            .snapshot_manager
            .get_collection_name()
            .await
            .context("No indexed codebase found.")?;

        // Drop collection
        self.vector_db.drop_collection(&collection_name).await?;

        // Clear snapshot
        self.snapshot_manager.clear().await;
        self.snapshot_manager.save().await?;

        Ok(vec![Content::Text {
            text: format!("Cleared index: {}", collection_name),
        }])
    }

    /// Handle get_indexing_status tool
    pub async fn handle_get_indexing_status(&self, args: &Value) -> Result<Vec<Content>> {
        let __path = args
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing 'path' argument")?;

        if let Some(collection_name) = self.snapshot_manager.get_collection_name().await {
            // Simple status without file count for now
            Ok(vec![Content::Text {
                text: format!("Status: Indexed\nCollection: {}", collection_name),
            }])
        } else {
            Ok(vec![Content::Text {
                text: "Status: Not indexed".to_string(),
            }])
        }
    }
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..max_len]
    }
}
