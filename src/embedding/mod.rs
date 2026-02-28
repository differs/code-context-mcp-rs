pub mod ollama;
pub mod openai;

use anyhow::Result;
use serde::Deserialize;

/// Embedding vector result
#[derive(Debug, Clone, Deserialize)]
pub struct Embedding {
    pub values: Vec<f32>,
}

/// Embedding provider trait
#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Embedding>;
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>>;
    fn dimension(&self) -> usize;
}
