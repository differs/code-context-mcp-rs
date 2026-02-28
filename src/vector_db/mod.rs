pub mod milvus;

use anyhow::Result;

/// Vector database trait
#[async_trait::async_trait]
pub trait VectorDatabase: Send + Sync {
    async fn create_collection(&self, name: &str, dimension: usize) -> Result<()>;
    async fn insert(&self, collection: &str, vectors: &[Vec<f32>], metadata: &[serde_json::Value]) -> Result<()>;
    async fn search(&self, collection: &str, vector: &[f32], limit: usize) -> Result<Vec<SearchResult>>;
    async fn drop_collection(&self, name: &str) -> Result<()>;
}

/// Search result from vector database
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub score: f32,
    pub metadata: serde_json::Value,
}
