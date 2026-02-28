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
    #[serde(rename = "collectionName")]
    collection_name: String,
    dimension: usize,
    #[serde(rename = "metricType")]
    metric_type: String,
}

#[derive(Debug, Serialize)]
struct InsertRequest {
    #[serde(rename = "collectionName")]
    collection_name: String,
    data: Vec<InsertData>,
}

#[derive(Debug, Serialize)]
struct InsertData {
    #[serde(rename = "id")]
    id: i64,
    #[serde(rename = "vector")]
    vector: Vec<f32>,
    #[serde(rename = "metadata")]
    metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct SearchRequest {
    #[serde(rename = "collectionName")]
    collection_name: String,
    data: Vec<Vec<f32>>,
    limit: usize,
    #[serde(rename = "outputFields")]
    output_fields: Vec<String>,
    #[serde(rename = "metricType")]
    metric_type: String,
}

#[derive(Debug, Deserialize)]
struct CreateCollectionResponse {
    code: i32,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    data: CreateCollectionData,
}

#[derive(Debug, Deserialize, Default)]
struct CreateCollectionData {
    #[serde(default)]
    collection_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    code: i32,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    cost: Option<i32>,
    data: Vec<SearchResultData>,
}

#[derive(Debug, Deserialize)]
struct SearchResultData {
    #[serde(rename = "distance", alias = "score")]
    score: f32,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
    #[serde(flatten)]
    extra: serde_json::Map<String, serde_json::Value>,
}

impl MilvusVectorDatabase {
    pub fn new(address: &str) -> Self {
        Self {
            client: Client::new(),
            address: address.trim_end_matches('/').to_string(),
        }
    }

    fn collection_url(&self) -> String {
        format!("{}/v2/vectordb/collections/create", self.address)
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

        let response_body: CreateCollectionResponse = response
            .json()
            .await
            .context("Failed to parse create collection response")?;

        if response_body.code != 0 {
            anyhow::bail!("Milvus create collection error: {}", response_body.message.unwrap_or_default());
        }

        // Log collection creation details
        if let Some(collection_name) = &response_body.data.collection_name {
            tracing::debug!("Created collection: {}", collection_name);
        } else {
            tracing::debug!("Created collection: {}", name);
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
            .enumerate()
            .map(|(i, (vector, meta))| InsertData {
                id: i as i64,
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

        let response_text = response.text().await.context("Failed to read search response")?;
        tracing::debug!("Milvus search raw response: {}", response_text);

        let search_response: SearchResponse = serde_json::from_str(&response_text)
            .with_context(|| format!("Failed to parse search response: {}", response_text))?;

        // Check for Milvus API error
        if search_response.code != 0 {
            anyhow::bail!("Milvus search error: {}", search_response.message.unwrap_or_default());
        }

        // Performance monitoring: log search cost
        let cost_ms = search_response.cost.unwrap_or(0);
        let result_count = search_response.data.len();
        
        if cost_ms > 100 {
            tracing::warn!(
                "Slow search detected: {}ms, collection={}, results={}",
                cost_ms, collection, result_count
            );
        } else {
            tracing::debug!(
                "Search completed: {}ms, collection={}, results={}",
                cost_ms, collection, result_count
            );
        }

        let results: Vec<SearchResult> = search_response
            .data
            .into_iter()
            .filter_map(|r| {
                // Metadata is directly in the result, or in extra fields
                let metadata = r.metadata.unwrap_or_else(|| serde_json::Value::Object(r.extra));
                
                Some(SearchResult {
                    score: r.score,
                    metadata,
                })
            })
            .collect();

        Ok(results)
    }

    async fn drop_collection(&self, name: &str) -> Result<()> {
        let request = json!({
            "collectionName": name
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
