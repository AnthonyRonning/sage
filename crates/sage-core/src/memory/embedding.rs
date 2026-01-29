//! Embedding Service
//!
//! Shared embedding generation for all memory tiers.
//! Uses Maple API with nomic-embed-text model (768 dimensions).

use anyhow::Result;
use tracing::warn;

/// Embedding dimension for nomic-embed-text
pub const EMBEDDING_DIM: usize = 768;

/// Shared embedding service for generating vector embeddings
#[derive(Clone)]
pub struct EmbeddingService {
    api_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl EmbeddingService {
    /// Create a new embedding service
    pub fn new(api_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            api_url: api_url.to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            client: reqwest::Client::new(),
        }
    }
    
    /// Generate an embedding for a single text
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let response = self.client
            .post(format!("{}/embeddings", self.api_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "model": &self.model,
                "input": text,
                "encoding_format": "float"  // Important: avoid base64 encoding issues
            }))
            .send()
            .await;
        
        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let json: serde_json::Value = resp.json().await?;
                    if let Some(embedding) = json["data"][0]["embedding"].as_array() {
                        let vec: Vec<f32> = embedding
                            .iter()
                            .filter_map(|v| v.as_f64().map(|f| f as f32))
                            .collect();
                        
                        if vec.len() == EMBEDDING_DIM {
                            return Ok(vec);
                        }
                        warn!("Unexpected embedding dimension: {} (expected {})", vec.len(), EMBEDDING_DIM);
                    }
                }
                warn!("Embedding API returned non-success status");
                Ok(zero_embedding())
            }
            Err(e) => {
                warn!("Failed to generate embedding: {}", e);
                Ok(zero_embedding())
            }
        }
    }
    
    /// Generate embeddings for multiple texts (batched)
    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        
        let response = self.client
            .post(format!("{}/embeddings", self.api_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "model": &self.model,
                "input": texts,
                "encoding_format": "float"
            }))
            .send()
            .await;
        
        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let json: serde_json::Value = resp.json().await?;
                    if let Some(data) = json["data"].as_array() {
                        let embeddings: Vec<Vec<f32>> = data
                            .iter()
                            .filter_map(|item| {
                                item["embedding"].as_array().map(|arr| {
                                    arr.iter()
                                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                                        .collect()
                                })
                            })
                            .collect();
                        
                        if embeddings.len() == texts.len() {
                            return Ok(embeddings);
                        }
                    }
                }
                warn!("Batch embedding API call failed, using zero embeddings");
                Ok(texts.iter().map(|_| zero_embedding()).collect())
            }
            Err(e) => {
                warn!("Failed to generate batch embeddings: {}", e);
                Ok(texts.iter().map(|_| zero_embedding()).collect())
            }
        }
    }
}

/// Return a zero embedding (fallback when API fails)
fn zero_embedding() -> Vec<f32> {
    vec![0.0; EMBEDDING_DIM]
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_zero_embedding() {
        let emb = zero_embedding();
        assert_eq!(emb.len(), EMBEDDING_DIM);
        assert!(emb.iter().all(|&x| x == 0.0));
    }
}
