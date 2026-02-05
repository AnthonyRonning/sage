//! GEPA-compatible module wrapper for Sage agent
//!
//! This wraps the Sage agent signatures to work with GEPA's optimization loop.

use super::{evaluate_rule_based, EvaluationResult, GepaConfig, GepaExample, ParsedResponse, ParsedToolCall};
use anyhow::Result;
use std::collections::HashMap;

/// A simplified agent module for GEPA optimization
///
/// This doesn't use the full SageAgent with database/tools, but focuses
/// on the core LLM prediction for instruction optimization.
pub struct GepaSageModule {
    /// Current instruction being evaluated
    pub instruction: String,
    /// Configuration
    pub config: GepaConfig,
}

impl GepaSageModule {
    pub fn new(config: GepaConfig) -> Self {
        Self {
            instruction: config.seed_instruction.clone(),
            config,
        }
    }

    /// Set the instruction (called by GEPA during optimization)
    pub fn set_instruction(&mut self, instruction: String) {
        self.instruction = instruction;
    }

    /// Get the current instruction
    pub fn get_instruction(&self) -> &str {
        &self.instruction
    }

    /// Run a single example through the module and get a response
    ///
    /// This uses dspy-rs Predict internally with the current instruction.
    pub async fn forward(&self, example: &GepaExample) -> Result<ParsedResponse> {
        use dspy_rs::{Predict, BamlType};

        // Define the signature inline (matches AgentResponse in sage_agent.rs)
        #[derive(dspy_rs::Signature, Clone, Debug)]
        struct GepaAgentResponse {
            #[input(desc = "The input to respond to")]
            input: String,
            #[input(desc = "Previous context summary")]
            previous_context_summary: String,
            #[input(desc = "Conversation context")]
            conversation_context: String,
            #[input(desc = "Available tools")]
            available_tools: String,

            #[output(desc = "Reasoning")]
            reasoning: String,
            #[output(desc = "Messages to send")]
            messages: Vec<String>,
            #[output(desc = "Tool calls")]
            tool_calls: Vec<GepaToolCall>,
        }

        #[derive(Clone, Debug, Default, BamlType)]
        struct GepaToolCall {
            name: String,
            args: HashMap<String, String>,
        }

        // Create predictor with current instruction
        let predictor = Predict::<GepaAgentResponse>::builder()
            .instruction(&self.instruction)
            .build();

        // Prepare input
        let input = GepaAgentResponseInput {
            input: example.input.clone(),
            previous_context_summary: example.previous_context_summary.clone(),
            conversation_context: example.conversation_context.clone(),
            available_tools: GEPA_TOOLS_DESCRIPTION.to_string(),
        };

        // Call LLM
        match predictor.call(input).await {
            Ok(response) => Ok(ParsedResponse {
                reasoning: response.reasoning,
                messages: response.messages,
                tool_calls: response
                    .tool_calls
                    .into_iter()
                    .map(|t| ParsedToolCall {
                        name: t.name,
                        args: t.args,
                    })
                    .collect(),
                parse_error: None,
            }),
            Err(e) => Ok(ParsedResponse {
                reasoning: String::new(),
                messages: vec![],
                tool_calls: vec![],
                parse_error: Some(e.to_string()),
            }),
        }
    }

    /// Evaluate a response against an example
    pub fn evaluate(&self, example: &GepaExample, response: &ParsedResponse) -> EvaluationResult {
        evaluate_rule_based(example, response)
    }

    /// Run an example through the module and evaluate
    pub async fn forward_and_evaluate(&self, example: &GepaExample) -> Result<EvaluationResult> {
        let response = self.forward(example).await?;
        Ok(self.evaluate(example, &response))
    }

    /// Evaluate on a batch of examples
    pub async fn evaluate_batch(&self, examples: &[GepaExample]) -> Result<Vec<EvaluationResult>> {
        let mut results = Vec::with_capacity(examples.len());
        for example in examples {
            let result = self.forward_and_evaluate(example).await?;
            results.push(result);
        }
        Ok(results)
    }

    /// Calculate average score across examples
    pub async fn average_score(&self, examples: &[GepaExample]) -> Result<f32> {
        let results = self.evaluate_batch(examples).await?;
        if results.is_empty() {
            return Ok(0.0);
        }
        let sum: f32 = results.iter().map(|r| r.score).sum();
        Ok(sum / results.len() as f32)
    }
}

/// Simplified tools description for GEPA evaluation
///
/// This is a subset of the real tools, focused on the patterns we want to evaluate.
const GEPA_TOOLS_DESCRIPTION: &str = r#"Available tools (add to tool_calls array to use):

web_search:
  Description: Search the web for current information
  Args: {"query": "search query"}

memory_append:
  Description: Add information to a memory block
  Args: {"block": "persona|human", "content": "text to add"}

memory_replace:
  Description: Replace text in a memory block (requires exact match)
  Args: {"block": "persona|human", "old_text": "exact text to find", "new_text": "replacement"}

archival_insert:
  Description: Store information in long-term archival memory
  Args: {"content": "text to store"}

archival_search:
  Description: Search archival memory semantically
  Args: {"query": "search query"}

conversation_search:
  Description: Search past conversation history
  Args: {"query": "search query"}

done:
  Description: Signal that nothing more needs to be done this turn
  Args: {}
"#;

/// Result of GEPA optimization
#[derive(Clone, Debug)]
pub struct GepaOptimizationResult {
    /// Best instruction found
    pub best_instruction: String,
    /// Best average score achieved
    pub best_score: f32,
    /// All candidate instructions evaluated
    pub all_candidates: Vec<(String, f32)>,
    /// Evolution history (generation, best score)
    pub evolution_history: Vec<(usize, f32)>,
    /// Total LLM calls made
    pub total_lm_calls: usize,
}

/// Run GEPA optimization (manual implementation without full dspy-rs GEPA)
///
/// This is a simplified version that demonstrates the optimization loop.
/// For production, you would use dspy-rs GEPA directly.
pub async fn run_optimization_simple(
    config: GepaConfig,
    trainset: Vec<GepaExample>,
    valset: Option<Vec<GepaExample>>,
) -> Result<GepaOptimizationResult> {
    use dspy_rs::{LM, configure, ChatAdapter};

    tracing::info!("Starting GEPA optimization");
    tracing::info!("  Iterations: {}", config.num_iterations);
    tracing::info!("  Minibatch size: {}", config.minibatch_size);
    tracing::info!("  Training examples: {}", trainset.len());

    // Configure LM (uses global config from environment)
    let lm = LM::builder()
        .temperature(config.temperature)
        .build()
        .await?;
    configure(lm, ChatAdapter);

    let eval_set = valset.as_ref().unwrap_or(&trainset);

    let mut module = GepaSageModule::new(config.clone());
    let mut best_instruction = config.seed_instruction.clone();
    let mut best_score: f32;
    let mut all_candidates = Vec::new();
    let mut evolution_history = Vec::new();
    let mut total_lm_calls = 0;

    // Evaluate seed instruction
    let seed_score = module.average_score(eval_set).await?;
    best_score = seed_score;
    all_candidates.push((best_instruction.clone(), seed_score));
    evolution_history.push((0, seed_score));
    total_lm_calls += eval_set.len();

    tracing::info!("Seed instruction score: {:.3}", seed_score);

    // Main optimization loop (simplified - just random mutations for now)
    // In production, use dspy-rs GEPA which has proper reflection
    for generation in 1..=config.num_iterations {
        tracing::info!("Generation {}/{}", generation, config.num_iterations);

        // Check budget
        if let Some(max_calls) = config.max_lm_calls {
            if total_lm_calls >= max_calls {
                tracing::info!("Budget limit reached");
                break;
            }
        }

        // Sample minibatch
        let minibatch: Vec<GepaExample> = trainset
            .iter()
            .take(config.minibatch_size)
            .cloned()
            .collect();

        // Collect feedback from current instruction
        let results = module.evaluate_batch(&minibatch).await?;
        total_lm_calls += minibatch.len();

        let feedback: Vec<String> = results
            .iter()
            .filter(|r| r.score < 0.8)
            .map(|r| r.feedback.clone())
            .collect();

        if !feedback.is_empty() {
            // Generate improved instruction through reflection
            // This is where dspy-rs GEPA would use ReflectOnTrace + ProposeImprovedInstruction
            let new_instruction = generate_improved_instruction(
                &module.instruction,
                &feedback,
            ).await?;
            total_lm_calls += 2; // Reflection + proposal

            module.set_instruction(new_instruction.clone());

            // Evaluate new instruction
            let new_score = module.average_score(eval_set).await?;
            total_lm_calls += eval_set.len();

            all_candidates.push((new_instruction.clone(), new_score));

            if new_score > best_score {
                best_score = new_score;
                best_instruction = new_instruction;
                tracing::info!("  New best score: {:.3}", new_score);
            } else {
                // Revert to best
                module.set_instruction(best_instruction.clone());
                tracing::info!("  Score {:.3} (no improvement)", new_score);
            }
        }

        evolution_history.push((generation, best_score));
    }

    tracing::info!("Optimization complete");
    tracing::info!("  Best score: {:.3}", best_score);
    tracing::info!("  Total LM calls: {}", total_lm_calls);

    Ok(GepaOptimizationResult {
        best_instruction,
        best_score,
        all_candidates,
        evolution_history,
        total_lm_calls,
    })
}

/// Generate an improved instruction using LLM reflection
async fn generate_improved_instruction(
    current_instruction: &str,
    feedback: &[String],
) -> Result<String> {
    use dspy_rs::{Predict, Signature};

    #[derive(Signature, Clone, Debug)]
    struct ImproveInstruction {
        #[input(desc = "The current instruction for the AI assistant")]
        current_instruction: String,

        #[input(desc = "Feedback from failed evaluations showing what went wrong")]
        feedback: String,

        #[output(desc = "An improved instruction that addresses the feedback")]
        improved_instruction: String,
    }

    let predictor = Predict::<ImproveInstruction>::builder()
        .instruction(
            "You are an expert prompt engineer. Given the current instruction and feedback \
             about failures, propose an improved instruction that addresses the issues. \
             Keep the instruction concise but comprehensive. Focus on the patterns that \
             caused failures.",
        )
        .build();

    let feedback_text = feedback.join("\n---\n");
    let input = ImproveInstructionInput {
        current_instruction: current_instruction.to_string(),
        feedback: feedback_text,
    };

    let result = predictor.call(input).await?;
    Ok(result.improved_instruction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_creation() {
        let config = GepaConfig::development();
        let module = GepaSageModule::new(config.clone());
        assert_eq!(module.get_instruction(), config.seed_instruction);
    }

    #[test]
    fn test_instruction_update() {
        let config = GepaConfig::development();
        let mut module = GepaSageModule::new(config);
        module.set_instruction("New instruction".to_string());
        assert_eq!(module.get_instruction(), "New instruction");
    }
}
