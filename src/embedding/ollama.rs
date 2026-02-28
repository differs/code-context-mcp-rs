use super::{Embedding, EmbeddingProvider};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Ollama embedding provider
pub struct OllamaEmbedding {
    client: Client,
    host: String,
    model: String,
    dimension: usize,
}

#[derive(Debug, Serialize)]
struct OllamaEmbeddingRequest {
    model: String,
    prompt: String,
}

#[derive(Debug, Deserialize)]
struct OllamaEmbeddingResponse {
    embedding: Vec<f32>,
}

impl OllamaEmbedding {
    pub fn new(host: &str, model: &str) -> Self {
        // Dimension depends on model - nomic-embed-text is 768
        let dimension = if model.contains("nomic") {
            768
        } else if model.contains("mxbai") {
            1024
        } else if model.contains("all-minilm") {
            384
        } else {
            768 // default
        };

        Self {
            client: Client::new(),
            host: host.trim_end_matches('/').to_string(),
            model: model.to_string(),
            dimension,
        }
    }

    async fn embed_single(&self, text: &str) -> Result<Embedding> {
        let url = format!("{}/api/embeddings", self.host);

        let request = OllamaEmbeddingRequest {
            model: self.model.clone(),
            prompt: text.to_string(),
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Ollama")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API error ({}): {}", status, body);
        }

        let embedding_response: OllamaEmbeddingResponse = response
            .json()
            .await
            .context("Failed to parse Ollama response")?;

        Ok(Embedding {
            values: embedding_response.embedding,
        })
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for OllamaEmbedding {
    async fn embed(&self, text: &str) -> Result<Embedding> {
        self.embed_single(text).await
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        // Ollama doesn't support batch embeddings, process sequentially
        let mut embeddings = Vec::with_capacity(texts.len());
        for text in texts {
            let embedding = self.embed_single(text).await?;
            embeddings.push(embedding);
        }
        Ok(embeddings)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}
