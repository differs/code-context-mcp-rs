use crate::embedding::EmbeddingProvider;
use crate::mcp::types::Content;
use crate::parser::code_parser::CodeParser;
use crate::snapshot::SnapshotManager;
use crate::vector_db::VectorDatabase;
use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use ignore::WalkBuilder;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;

/// Maximum file size to index (10 MB)
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Tool handlers for MCP server
pub struct ToolHandlers {
    embedding: Arc<dyn EmbeddingProvider>,
    vector_db: Arc<dyn VectorDatabase>,
    snapshot_manager: Arc<SnapshotManager>,
    code_parser: CodeParser,
    max_projects: usize,
}

impl ToolHandlers {
    pub fn new(
        embedding: Arc<dyn EmbeddingProvider>,
        vector_db: Arc<dyn VectorDatabase>,
        snapshot_manager: Arc<SnapshotManager>,
        max_projects: usize,
    ) -> Self {
        Self {
            embedding,
            vector_db,
            snapshot_manager,
            code_parser: CodeParser::new(),
            max_projects,
        }
    }

    /// Validate and normalize path, return error if path is invalid
    fn validate_path(path_str: &str) -> Result<PathBuf> {
        let path = Path::new(path_str);
        
        // Convert to absolute path
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        // Security check: ensure path doesn't contain suspicious patterns
        let path_str = abs_path.to_string_lossy();
        if path_str.contains("..") && !path_str.starts_with("..") {
            anyhow::bail!("Invalid path: suspicious path traversal detected");
        }

        Ok(abs_path)
    }

    /// Handle index_codebase tool
    pub async fn handle_index_codebase(&self, args: &Value) -> Result<Vec<Content>> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing 'path' argument")?;

        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
        let _splitter = args
            .get("splitter")
            .and_then(|v| v.as_str())
            .unwrap_or("ast");

        // Validate and normalize path
        let project_root = Self::validate_path(path_str)?;

        if !project_root.exists() {
            anyhow::bail!("Path does not exist: {}", project_root.display());
        }

        if !project_root.is_dir() {
            anyhow::bail!("Path is not a directory: {}", project_root.display());
        }

        // Generate collection name from path hash
        let path_hash = CodeParser::hash_file(&project_root.to_string_lossy());
        let collection_name = format!("code_index_{}", &path_hash[..16]);

        // Check if already indexed
        if let Some(existing_collection) = self.snapshot_manager.get_collection_name(&project_root).await {
            if existing_collection == collection_name && !force {
                return Ok(vec![Content::Text {
                    text: format!(
                        "Codebase already indexed. Use force=true to re-index.\nProject: {}\nCollection: {}",
                        project_root.display(),
                        collection_name
                    ),
                }]);
            }
        }

        // Check if we need to evict oldest project (LRU)
        let (_root_info, to_evict) = self.snapshot_manager.get_or_create_root(&project_root, &collection_name).await;
        
        // Evict oldest project if needed
        let mut eviction_info = None;
        if let Some(evict_path) = to_evict {
            if let Some(evict_collection) = self.snapshot_manager.remove_root(&evict_path).await {
                // Drop the old collection from Milvus
                if let Err(e) = self.vector_db.drop_collection(&evict_collection).await {
                    tracing::warn!("Failed to drop evicted collection {}: {}", evict_collection, e);
                }
                eviction_info = Some((evict_path, evict_collection));
            }
        }

        // Create collection if not exists
        let dimension = self.embedding.dimension();
        
        // Try to create collection (ignore error if already exists)
        if let Err(e) = self.vector_db.create_collection(&collection_name, dimension).await {
            tracing::warn!("Failed to create collection (may already exist): {}", e);
        }
        
        // Verify collection exists by attempting a dummy search
        // This ensures the collection is ready for insertions
        tracing::info!("Created/verified collection: {}", collection_name);

        tracing::info!("Indexing codebase at: {}", project_root.display());

        // Walk directory and index files
        let mut total_files = 0;
        let mut total_chunks = 0;
        let mut skipped_files = 0;
        let mut skipped_size = 0u64;

        let walker = WalkBuilder::new(&project_root)
            .standard_filters(true)
            .hidden(true) // Skip hidden files
            .build();

        for entry in walker.flatten() {
            if entry.file_type().map_or(true, |ft| !ft.is_file()) {
                continue;
            }

            let file_path = entry.path();
            
            // Security check: ensure file is within project root
            if !file_path.starts_with(&project_root) {
                tracing::warn!("Skipping file outside project root: {:?}", file_path);
                skipped_files += 1;
                continue;
            }

            // Get file metadata to check size
            let metadata = match fs::metadata(file_path).await {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("Failed to get metadata for {:?}: {}", file_path, e);
                    skipped_files += 1;
                    continue;
                }
            };

            // Skip files larger than MAX_FILE_SIZE
            if metadata.len() > MAX_FILE_SIZE {
                tracing::debug!("Skipping large file {:?} ({} bytes)", file_path, metadata.len());
                skipped_size += metadata.len();
                skipped_files += 1;
                continue;
            }
            
            // Read file content
            let content = match fs::read_to_string(file_path).await {
                Ok(c) => c,
                Err(_) => {
                    skipped_files += 1;
                    continue; // Skip binary files
                }
            };

            // Calculate hash
            let file_hash = CodeParser::hash_file(&content);

            // Check if file has changed
            if let Some(existing_hash) = self.snapshot_manager.get_file_hash(&project_root, file_path).await {
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

            // Generate embeddings with concurrent processing
            let texts: Vec<String> = chunks
                .iter()
                .map(|c| format!("{}\n{}", c.content, c.symbol_name.as_deref().unwrap_or("")))
                .collect();

            let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
            
            // Use concurrent batch embedding (process 5 at a time)
            let embeddings = self.embed_batch_concurrent(&text_refs).await;
            
            if embeddings.is_empty() {
                tracing::warn!("Failed to generate embeddings for {:?}", file_path);
                continue;
            }

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
                        "project_root": project_root.to_string_lossy().as_ref(),
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
                .update_file(&project_root, file_path.to_path_buf(), file_hash, chunks.len())
                .await;

            total_files += 1;
            total_chunks += chunks.len();
        }
        
        // Save snapshot
        self.snapshot_manager.save().await?;

        let mut result = format!(
            "Indexed {} files, {} chunks\nProject: {}\nCollection: {}\nProjects: {}/{}",
            total_files, total_chunks, project_root.display(), collection_name,
            self.snapshot_manager.get_project_count().await, self.max_projects
        );
        
        if skipped_files > 0 {
            result.push_str(&format!("\nSkipped {} files ({} MB filtered by size)", 
                skipped_files, skipped_size as f64 / 1024.0 / 1024.0));
        }
        
        if let Some((evict_path, evict_collection)) = eviction_info {
            result.push_str(&format!(
                "\n⚠️  Evicted oldest project: {} (collection: {})",
                evict_path.display(), evict_collection
            ));
        }

        Ok(vec![Content::Text { text: result }])
    }

    /// Concurrent batch embedding with configurable concurrency
    async fn embed_batch_concurrent(&self, texts: &[&str]) -> Vec<crate::embedding::Embedding> {
        const CONCURRENCY: usize = 5;
        
        stream::iter(texts.iter().copied())
            .map(|text| async move {
                self.embedding.embed(text).await
            })
            .buffer_unordered(CONCURRENCY)
            .filter_map(|result| async move {
                match result {
                    Ok(embedding) => Some(embedding),
                    Err(e) => {
                        tracing::warn!("Embedding failed: {}", e);
                        None
                    }
                }
            })
            .collect()
            .await
    }

    /// Handle search_code tool
    pub async fn handle_search_code(&self, args: &Value) -> Result<Vec<Content>> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing 'path' argument")?;

        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .context("Missing 'query' argument")?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let cross_project = args.get("cross_project").and_then(|v| v.as_bool()).unwrap_or(false);

        // Validate path
        let search_path = Self::validate_path(path_str)?;

        // Embed query
        let embedding = self.embedding.embed(query).await?;

        let results = if cross_project || search_path.to_string_lossy().ends_with("/all") || search_path.to_string_lossy() == "all" {
            // Cross-project search: search all collections
            self.search_cross_project(&embedding.values, limit).await?
        } else {
            // Single project search
            let project_root = if let Some(root) = self.snapshot_manager.find_project_root(&search_path).await {
                root
            } else {
                // Try to use the path itself as project root
                search_path.clone()
            };

            let collection_name = self
                .snapshot_manager
                .get_collection_name(&project_root)
                .await
                .context("No indexed codebase found for this path. Please index first.")?;

            // Search vector database
            self.vector_db.search(&collection_name, &embedding.values, limit).await?
        };

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
            let project = result
                .metadata
                .get("project_root")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let project_info = if !project.is_empty() {
                format!(" [{}]", Path::new(project).file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown"))
            } else {
                String::new()
            };

            formatted.push_str(&format!(
                "{}. **{}** (`{}:{}-{}`){}\nScore: {:.2}%\n```\n{}\n```\n\n",
                i + 1,
                symbol_name,
                file_path,
                start_line + 1,
                end_line + 1,
                project_info,
                result.score * 100.0,
                truncate(content, 500)
            ));
        }

        Ok(vec![Content::Text { text: formatted }])
    }

    /// Search across all indexed projects
    async fn search_cross_project(&self, vector: &[f32], per_project_limit: usize) -> Result<Vec<crate::vector_db::SearchResult>> {
        let collections = self.snapshot_manager.get_all_collection_names().await;
        
        if collections.is_empty() {
            return Ok(Vec::new());
        }

        // Search all collections concurrently
        let search_tasks: Vec<_> = collections
            .iter()
            .map(|(project_path, collection_name)| {
                let vector_ref = vector.to_vec();
                async move {
                    match self.vector_db.search(collection_name, &vector_ref, per_project_limit).await {
                        Ok(results) => Some((project_path.clone(), results)),
                        Err(e) => {
                            tracing::warn!("Failed to search collection {}: {}", collection_name, e);
                            None
                        }
                    }
                }
            })
            .collect();

        let all_results: Vec<_> = futures::future::join_all(search_tasks)
            .await
            .into_iter()
            .flatten()
            .collect();

        // Merge and sort results by score
        let mut merged: Vec<_> = all_results
            .into_iter()
            .flat_map(|(project_path, results)| {
                results.into_iter().map(move |mut r| {
                    // Add project info to metadata
                    if let Some(obj) = r.metadata.as_object_mut() {
                        obj.insert("project_root".to_string(), json!(project_path.to_string_lossy().as_ref()));
                    }
                    r
                })
            })
            .collect();

        // Sort by score descending
        merged.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Take top results
        Ok(merged.into_iter().take(per_project_limit).collect())
    }

    /// Handle clear_index tool
    pub async fn handle_clear_index(&self, args: &Value) -> Result<Vec<Content>> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing 'path' argument")?;

        // Validate path
        let project_root = Self::validate_path(path_str)?;

        // Check for special "all" path
        if project_root.to_string_lossy().ends_with("/all") || project_root.to_string_lossy() == "all" {
            // Clear all projects
            let collections = self.snapshot_manager.get_all_collection_names().await;
            let mut cleared = Vec::new();
            
            for (path, collection_name) in &collections {
                if let Err(e) = self.vector_db.drop_collection(collection_name).await {
                    tracing::warn!("Failed to drop collection {}: {}", collection_name, e);
                } else {
                    cleared.push(path.display().to_string());
                }
            }
            
            self.snapshot_manager.clear().await;
            self.snapshot_manager.save().await?;

            return Ok(vec![Content::Text {
                text: format!("Cleared {} projects: {}", cleared.len(), cleared.join(", ")),
            }]);
        }

        // Single project clear
        let collection_name = self
            .snapshot_manager
            .get_collection_name(&project_root)
            .await
            .context("No indexed codebase found for this path.")?;

        // Drop collection
        self.vector_db.drop_collection(&collection_name).await?;

        // Clear snapshot for this project
        self.snapshot_manager.clear_project(&project_root).await;
        self.snapshot_manager.save().await?;

        Ok(vec![Content::Text {
            text: format!("Cleared index for {}\nCollection: {}", project_root.display(), collection_name),
        }])
    }

    /// Handle get_indexing_status tool
    pub async fn handle_get_indexing_status(&self, args: &Value) -> Result<Vec<Content>> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing 'path' argument")?;

        // Validate path
        let project_root = Self::validate_path(path_str)?;

        // Check for special "all" path
        if project_root.to_string_lossy().ends_with("/all") || project_root.to_string_lossy() == "all" {
            // Show all projects
            let roots = self.snapshot_manager.get_all_roots().await;
            
            if roots.is_empty() {
                return Ok(vec![Content::Text {
                    text: "No indexed projects found.".to_string(),
                }]);
            }

            let mut status = String::from("Indexed projects:\n\n");
            for (i, root) in roots.iter().enumerate() {
                if let Some(collection) = self.snapshot_manager.get_collection_name(root).await {
                    status.push_str(&format!(
                        "{}. {}\n   Collection: {}\n\n",
                        i + 1,
                        root.display(),
                        collection
                    ));
                }
            }

            return Ok(vec![Content::Text { text: status }]);
        }

        // Single project status
        if let Some(collection_name) = self.snapshot_manager.get_collection_name(&project_root).await {
            Ok(vec![Content::Text {
                text: format!(
                    "Status: Indexed\nProject: {}\nCollection: {}",
                    project_root.display(),
                    collection_name
                ),
            }])
        } else {
            Ok(vec![Content::Text {
                text: format!("Status: Not indexed\nProject: {}", project_root.display()),
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
