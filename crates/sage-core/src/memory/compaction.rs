//! Compaction / Summary Memory
//!
//! Summarizes old messages when context window approaches its limit.
//! Uses DSRs signature for summarization to enable GEPA optimization.

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use dspy_rs::{Predict, Signature};

/// Instruction for summarization DSRs signature
pub const SUMMARY_INSTRUCTION: &str = r#"You are a conversation summarizer. Your job is to create a concise summary that allows an AI agent to resume a conversation without disruption, even after older messages are replaced with this summary.

Your summary should be structured and actionable. Include:
1. Task/Conversation overview: What is the user working on? Key clarifications or constraints?
2. Current State: What has been completed or discussed? Any files/resources referenced?
3. Next Steps: What would logically come next in this conversation?

Keep your summary under 100 words. Be specific and preserve key details like names, preferences, and decisions made."#;

/// Instruction for correction DSRs signature
pub const CORRECTION_INSTRUCTION: &str = r#"You are a correction agent. The summarizer produced a malformed response that couldn't be parsed. Your job is to extract the summary from the malformed response and return it in the correct format.

Preserve the original intent and content - do NOT generate new content. Just reshape the malformed response into the expected output format."#;

/// DSRs signature for conversation summarization
#[derive(Signature, Clone, Debug)]
pub struct SummarizeConversation {
    #[input(desc = "Previous summary to build upon (empty if first summarization)")]
    pub previous_summary: String,

    #[input(desc = "New conversation messages to incorporate into the summary")]
    pub new_messages: String,

    #[output(desc = "Updated summary incorporating all context (100 word limit)")]
    pub summary: String,
}

/// DSRs signature for correcting malformed summarization responses
#[derive(Signature, Clone, Debug)]
pub struct SummarizationCorrection {
    #[input(desc = "The previous summary that was being built upon")]
    pub previous_summary: String,

    #[input(desc = "The new messages that were being summarized")]
    pub new_messages: String,

    #[input(desc = "The malformed response that needs correction")]
    pub malformed_response: String,

    #[input(desc = "The error message explaining what went wrong")]
    pub error_message: String,

    #[output(desc = "Corrected summary (100 word limit)")]
    pub summary: String,
}

/// Result of a summarization operation
#[derive(Debug, Clone)]
pub struct SummaryResult {
    pub id: Uuid,
    pub summary: String,
    pub from_sequence_id: i64,
    pub to_sequence_id: i64,
    pub previous_summary_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl SummaryResult {
    pub fn new(
        summary: impl Into<String>,
        from_sequence_id: i64,
        to_sequence_id: i64,
        previous_summary_id: Option<Uuid>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            summary: summary.into(),
            from_sequence_id,
            to_sequence_id,
            previous_summary_id,
            created_at: Utc::now(),
        }
    }
}

/// Manages compaction/summarization with retry and correction support
pub struct CompactionManager {
    max_retries: usize,
}

impl CompactionManager {
    pub fn new() -> Self {
        Self { max_retries: 2 }
    }

    /// Summarize messages with automatic retry and correction on failure
    pub async fn summarize(
        &self,
        previous_summary: &str,
        new_messages: &str,
        from_sequence_id: i64,
        to_sequence_id: i64,
        previous_summary_id: Option<Uuid>,
    ) -> Result<SummaryResult> {
        let predictor = Predict::<SummarizeConversation>::builder()
            .instruction(SUMMARY_INSTRUCTION)
            .build();

        let input = SummarizeConversationInput {
            previous_summary: previous_summary.to_string(),
            new_messages: new_messages.to_string(),
        };

        // First attempt
        match predictor.call(input.clone()).await {
            Ok(response) => {
                tracing::info!("Summarization succeeded on first attempt");
                return Ok(SummaryResult::new(
                    response.summary,
                    from_sequence_id,
                    to_sequence_id,
                    previous_summary_id,
                ));
            }
            Err(e) => {
                tracing::warn!("Summarization failed, attempting correction: {}", e);
                
                // Try correction agent
                if let Some(malformed) = extract_malformed_response(&e) {
                    if let Ok(corrected) = self.try_correction(
                        previous_summary,
                        new_messages,
                        &malformed,
                        &e.to_string(),
                    ).await {
                        return Ok(SummaryResult::new(
                            corrected,
                            from_sequence_id,
                            to_sequence_id,
                            previous_summary_id,
                        ));
                    }
                }
            }
        }

        // Retry loop
        for attempt in 1..=self.max_retries {
            tracing::info!("Summarization retry attempt {}/{}", attempt, self.max_retries);
            
            match predictor.call(input.clone()).await {
                Ok(response) => {
                    tracing::info!("Summarization succeeded on retry {}", attempt);
                    return Ok(SummaryResult::new(
                        response.summary,
                        from_sequence_id,
                        to_sequence_id,
                        previous_summary_id,
                    ));
                }
                Err(e) => {
                    tracing::warn!("Summarization retry {} failed: {}", attempt, e);
                    
                    // Try correction on each failure
                    if let Some(malformed) = extract_malformed_response(&e) {
                        if let Ok(corrected) = self.try_correction(
                            previous_summary,
                            new_messages,
                            &malformed,
                            &e.to_string(),
                        ).await {
                            return Ok(SummaryResult::new(
                                corrected,
                                from_sequence_id,
                                to_sequence_id,
                                previous_summary_id,
                            ));
                        }
                    }
                }
            }
        }

        anyhow::bail!("Summarization failed after {} retries", self.max_retries)
    }

    /// Try to correct a malformed response using the correction agent
    async fn try_correction(
        &self,
        previous_summary: &str,
        new_messages: &str,
        malformed_response: &str,
        error_message: &str,
    ) -> Result<String> {
        tracing::info!("Attempting summarization correction");

        let correction_predictor = Predict::<SummarizationCorrection>::builder()
            .instruction(CORRECTION_INSTRUCTION)
            .build();

        let correction_input = SummarizationCorrectionInput {
            previous_summary: previous_summary.to_string(),
            new_messages: new_messages.to_string(),
            malformed_response: malformed_response.to_string(),
            error_message: error_message.to_string(),
        };

        let corrected = correction_predictor.call(correction_input).await?;
        tracing::info!("Summarization correction succeeded");
        Ok(corrected.summary)
    }

    /// Check if compaction is needed based on token count
    pub fn should_compact(&self, current_tokens: usize, max_tokens: usize, threshold: f32) -> bool {
        current_tokens > ((max_tokens as f32 * threshold) as usize)
    }
}

impl Default for CompactionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract malformed response from error if available
fn extract_malformed_response<E: std::fmt::Display>(error: &E) -> Option<String> {
    let error_str = error.to_string();
    // Look for common patterns that indicate raw LLM output in error
    if error_str.contains("Failed to parse") || error_str.contains("missing field") {
        // The error often contains the raw response
        Some(error_str)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summary_result_creation() {
        let summary = SummaryResult::new("Test summary", 1, 10, None);
        
        assert_eq!(summary.summary, "Test summary");
        assert_eq!(summary.from_sequence_id, 1);
        assert_eq!(summary.to_sequence_id, 10);
        assert!(summary.previous_summary_id.is_none());
    }

    #[test]
    fn test_summary_result_with_previous() {
        let prev_id = Uuid::new_v4();
        let summary = SummaryResult::new("Chained summary", 11, 20, Some(prev_id));
        
        assert_eq!(summary.from_sequence_id, 11);
        assert_eq!(summary.to_sequence_id, 20);
        assert_eq!(summary.previous_summary_id, Some(prev_id));
    }

    #[test]
    fn test_should_compact() {
        let manager = CompactionManager::new();
        
        // 80% threshold
        assert!(!manager.should_compact(50_000, 256_000, 0.80)); // 50k < 204k
        assert!(manager.should_compact(210_000, 256_000, 0.80)); // 210k > 204k
        assert!(manager.should_compact(256_000, 256_000, 0.80)); // 256k > 204k
    }
}
