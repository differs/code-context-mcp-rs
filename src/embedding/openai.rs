//! OpenAI Embedding provider (reserved for future use)
#![allow(dead_code)]

use super::{Embedding, EmbeddingProvider};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// OpenAI embedding provider
pub struct OpenAIEmbedding {
    client: Client,
    api_key: String,
    model: String,
    dimension: usize,
}

#[derive(Debug, Serialize)]
struct OpenAIEmbeddingRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<EmbeddingData>,
    usage: Usage,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: usize,
    total_tokens: usize,
}

impl OpenAIEmbedding {
    pub fn new(api_key: &str, model: &str) -> Self {
        let dimension = match model {
            "text-embedding-3-small" => 1536,
            "text-embedding-3-large" => 3072,
            "text-embedding-ada-002" => 1536,
            _ => 1536,
        };

        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            dimension,
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for OpenAIEmbedding {
    async fn embed(&self, text: &str) -> Result<Embedding> {
        let embeddings = self.embed_batch(&[text]).await?;
        embeddings
            .into_iter()
            .next()
            .context("No embedding returned")
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        let url = "https://api.openai.com/v1/embeddings";

        let request = OpenAIEmbeddingRequest {
            model: self.model.clone(),
            input: texts.iter().map(|s| s.to_string()).collect(),
        };

        let response = self
            .client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request)
            .send()
            .await
            .context("Failed to send request to OpenAI")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error ({}): {}", status, body);
        }

        let embedding_response: OpenAIEmbeddingResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI response")?;

        // Sort by index to maintain order
        let mut sorted_data = embedding_response.data;
        sorted_data.sort_by_key(|d| d.index);

        let embeddings: Vec<Embedding> = sorted_data
            .into_iter()
            .map(|d| Embedding {
                values: d.embedding,
            })
            .collect();

        Ok(embeddings)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}
