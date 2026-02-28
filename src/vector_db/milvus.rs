use super::{SearchResult, VectorDatabase};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Milvus vector database client (using REST API)
pub struct MilvusVectorDatabase {
    client: Client,
    address: String,
}

#[derive(Debug, Serialize)]
struct CreateCollectionRequest {
    collection_name: String,
    dimension: usize,
    metric_type: String,
}

#[derive(Debug, Serialize)]
struct InsertRequest {
    collection_name: String,
    data: Vec<InsertData>,
}

#[derive(Debug, Serialize)]
struct InsertData {
    vector: Vec<f32>,
    metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct SearchRequest {
    collection_name: String,
    data: Vec<Vec<f32>>,
    limit: usize,
    output_fields: Vec<String>,
    metric_type: String,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    data: Vec<SearchResultData>,
}

#[derive(Debug, Deserialize)]
struct SearchResultData {
    score: f32,
    entity: Entity,
}

#[derive(Debug, Deserialize)]
struct Entity {
    metadata: serde_json::Value,
}

impl MilvusVectorDatabase {
    pub fn new(address: &str) -> Self {
        Self {
            client: Client::new(),
            address: address.trim_end_matches('/').to_string(),
        }
    }

    fn collection_url(&self) -> String {
        format!("{}/v2/vectordb/collections", self.address)
    }

    fn insert_url(&self) -> String {
        format!("{}/v2/vectordb/entities/insert", self.address)
    }

    fn search_url(&self) -> String {
        format!("{}/v2/vectordb/entities/search", self.address)
    }

    fn drop_url(&self) -> String {
        format!("{}/v2/vectordb/collections/drop", self.address)
    }
}

#[async_trait::async_trait]
impl VectorDatabase for MilvusVectorDatabase {
    async fn create_collection(&self, name: &str, dimension: usize) -> Result<()> {
        let request = CreateCollectionRequest {
            collection_name: name.to_string(),
            dimension,
            metric_type: "COSINE".to_string(),
        };

        let response = self
            .client
            .post(self.collection_url())
            .json(&request)
            .send()
            .await
            .context("Failed to send create collection request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Milvus API error ({}): {}", status, body);
        }

        Ok(())
    }

    async fn insert(&self, collection: &str, vectors: &[Vec<f32>], metadata: &[serde_json::Value]) -> Result<()> {
        if vectors.len() != metadata.len() {
            anyhow::bail!("Vectors and metadata length mismatch");
        }

        let data: Vec<InsertData> = vectors
            .iter()
            .zip(metadata.iter())
            .map(|(vector, meta)| InsertData {
                vector: vector.clone(),
                metadata: meta.clone(),
            })
            .collect();

        let request = InsertRequest {
            collection_name: collection.to_string(),
            data,
        };

        let response = self
            .client
            .post(self.insert_url())
            .json(&request)
            .send()
            .await
            .context("Failed to send insert request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Milvus API error ({}): {}", status, body);
        }

        Ok(())
    }

    async fn search(&self, collection: &str, vector: &[f32], limit: usize) -> Result<Vec<SearchResult>> {
        let request = SearchRequest {
            collection_name: collection.to_string(),
            data: vec![vector.to_vec()],
            limit,
            output_fields: vec!["metadata".to_string()],
            metric_type: "COSINE".to_string(),
        };

        let response = self
            .client
            .post(self.search_url())
            .json(&request)
            .send()
            .await
            .context("Failed to send search request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Milvus API error ({}): {}", status, body);
        }

        let search_response: SearchResponse = response
            .json()
            .await
            .context("Failed to parse search response")?;

        let results: Vec<SearchResult> = search_response
            .data
            .into_iter()
            .map(|r| SearchResult {
                score: r.score,
                metadata: r.entity.metadata,
            })
            .collect();

        Ok(results)
    }

    async fn drop_collection(&self, name: &str) -> Result<()> {
        let request = json!({
            "collection_name": name
        });

        let response = self
            .client
            .post(self.drop_url())
            .json(&request)
            .send()
            .await
            .context("Failed to send drop collection request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Milvus API error ({}): {}", status, body);
        }

        Ok(())
    }
}
