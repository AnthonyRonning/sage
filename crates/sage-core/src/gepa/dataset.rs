//! Dataset loading and management for GEPA evaluation

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single evaluation example
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GepaExample {
    /// Unique identifier for this example
    pub id: String,
    /// Category (casual_chat, information_request, memory_storage, etc.)
    pub category: String,
    /// The input message (user message or tool result)
    pub input: String,
    /// Previous context summary (for long conversations)
    #[serde(default)]
    pub previous_context_summary: String,
    /// Recent conversation context
    pub conversation_context: String,
    /// Description of expected behavior
    pub expected_behavior: String,
    /// Type of expected response (casual, detailed, tool_use, silent_done, etc.)
    pub expected_response_type: String,
    /// List of tool names that should be called
    #[serde(default)]
    pub expected_tools: Vec<String>,
    /// Whether memory storage is expected
    #[serde(default)]
    pub should_store_memory: bool,
    /// Patterns that indicate a bad response
    #[serde(default)]
    pub bad_patterns: Vec<String>,
}

/// Dataset metadata
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GepaDataset {
    /// Description of the dataset
    pub description: String,
    /// Dataset version
    pub version: String,
    /// The examples
    pub examples: Vec<GepaExample>,
}

impl GepaDataset {
    /// Load dataset from a JSON file
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())?;
        let dataset: GepaDataset = serde_json::from_str(&content)?;
        Ok(dataset)
    }

    /// Save dataset to a JSON file
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Get examples by category
    pub fn filter_by_category(&self, category: &str) -> Vec<&GepaExample> {
        self.examples
            .iter()
            .filter(|e| e.category == category)
            .collect()
    }

    /// Get all unique categories
    pub fn categories(&self) -> Vec<String> {
        let mut cats: Vec<String> = self.examples.iter().map(|e| e.category.clone()).collect();
        cats.sort();
        cats.dedup();
        cats
    }

    /// Sample a random subset of examples
    pub fn sample(&self, n: usize) -> Vec<&GepaExample> {
        use std::collections::HashSet;

        if n >= self.examples.len() {
            return self.examples.iter().collect();
        }

        let mut rng = rand_simple();
        let mut indices: HashSet<usize> = HashSet::new();

        while indices.len() < n {
            let idx = rng.next_usize() % self.examples.len();
            indices.insert(idx);
        }

        indices
            .into_iter()
            .map(|i| &self.examples[i])
            .collect()
    }
}

/// Simple random number generator (no external dependency)
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_usize(&mut self) -> usize {
        self.next_u64() as usize
    }
}

fn rand_simple() -> SimpleRng {
    // Use current time as seed
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(12345);
    SimpleRng::new(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_trainset() {
        // This test requires the trainset file to exist
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/gepa/trainset.json");
        if std::path::Path::new(path).exists() {
            let dataset = GepaDataset::load_from_file(path).unwrap();
            assert!(!dataset.examples.is_empty());
            assert!(dataset.examples.len() >= 20);
        }
    }

    #[test]
    fn test_categories() {
        let dataset = GepaDataset {
            description: "Test".to_string(),
            version: "1.0".to_string(),
            examples: vec![
                GepaExample {
                    id: "1".to_string(),
                    category: "casual_chat".to_string(),
                    input: "Hi".to_string(),
                    previous_context_summary: "".to_string(),
                    conversation_context: "".to_string(),
                    expected_behavior: "Greet".to_string(),
                    expected_response_type: "casual".to_string(),
                    expected_tools: vec![],
                    should_store_memory: false,
                    bad_patterns: vec![],
                },
                GepaExample {
                    id: "2".to_string(),
                    category: "tool_use".to_string(),
                    input: "Search for X".to_string(),
                    previous_context_summary: "".to_string(),
                    conversation_context: "".to_string(),
                    expected_behavior: "Use search".to_string(),
                    expected_response_type: "tool_use".to_string(),
                    expected_tools: vec!["web_search".to_string()],
                    should_store_memory: false,
                    bad_patterns: vec![],
                },
            ],
        };

        let cats = dataset.categories();
        assert_eq!(cats.len(), 2);
        assert!(cats.contains(&"casual_chat".to_string()));
        assert!(cats.contains(&"tool_use".to_string()));
    }
}
