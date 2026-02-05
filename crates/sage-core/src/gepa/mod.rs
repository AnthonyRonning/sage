//! GEPA (Genetic-Pareto) Prompt Optimization for Sage
//!
//! This module provides tools for optimizing Sage's agent instructions
//! using GEPA's reflective prompt evolution algorithm.
//!
//! ## Overview
//!
//! GEPA works by:
//! 1. Starting with a seed instruction (can be minimal or detailed)
//! 2. Running examples through the agent and collecting feedback
//! 3. Using an LLM to reflect on failures and propose improvements
//! 4. Evaluating improved instructions on a validation set
//! 5. Maintaining a Pareto frontier of candidates with diverse strengths
//!
//! ## Usage
//!
//! ```rust,ignore
//! use sage_core::gepa::{GepaConfig, GepaSageModule, run_optimization};
//!
//! let config = GepaConfig::development();
//! let trainset = load_dataset("examples/gepa/trainset.json")?;
//! let result = run_optimization(config, trainset).await?;
//!
//! println!("Best instruction:\n{}", result.best_candidate.instruction);
//! ```

// Allow dead code - this module is experimental and not yet fully integrated
#![allow(dead_code)]

pub mod dataset;
pub mod evaluator;
pub mod module;

#[allow(unused_imports)]
pub use dataset::*;
#[allow(unused_imports)]
pub use evaluator::*;
#[allow(unused_imports)]
pub use module::*;

/// Minimal seed instruction for GEPA optimization
///
/// This gives GEPA room to explore the prompt space rather than
/// being constrained by our manual engineering.
pub const MINIMAL_SEED_INSTRUCTION: &str = r#"You are Sage, a helpful AI assistant communicating via Signal.

You have access to memory (core blocks, archival storage) and tools (web search, etc.).
Respond naturally. Use tools when helpful. Store important information to memory proactively.

For casual chat, use multiple short messages. For detailed explanations, longer messages are fine.
After memory operations complete, return done silently (no announcement needed)."#;

/// Configuration for GEPA optimization
#[derive(Clone, Debug)]
pub struct GepaConfig {
    /// Number of evolutionary iterations
    pub num_iterations: usize,
    /// Examples per iteration
    pub minibatch_size: usize,
    /// Trials per candidate evaluation
    pub num_trials: usize,
    /// Temperature for LLM mutations
    pub temperature: f32,
    /// Track detailed statistics
    pub track_stats: bool,
    /// Maximum total rollouts (budget control)
    pub max_rollouts: Option<usize>,
    /// Maximum LM calls (budget control)
    pub max_lm_calls: Option<usize>,
    /// Model for reflection/mutation (e.g., "claude-sonnet-4-5-20250514")
    pub prompt_model: Option<String>,
    /// Model for judge evaluation (if using LLM-as-judge)
    pub judge_model: Option<String>,
    /// Whether to use LLM-as-judge (more nuanced but expensive)
    pub use_llm_judge: bool,
    /// Seed instruction (defaults to MINIMAL_SEED_INSTRUCTION)
    pub seed_instruction: String,
}

impl Default for GepaConfig {
    fn default() -> Self {
        Self {
            num_iterations: 15,
            minibatch_size: 10,
            num_trials: 5,
            temperature: 0.9,
            track_stats: true,
            max_rollouts: Some(500),
            max_lm_calls: Some(1000),
            prompt_model: None,
            judge_model: None,
            use_llm_judge: false,
            seed_instruction: MINIMAL_SEED_INSTRUCTION.to_string(),
        }
    }
}

impl GepaConfig {
    /// Quick development configuration (fast, cheap)
    pub fn development() -> Self {
        Self {
            num_iterations: 5,
            minibatch_size: 5,
            num_trials: 2,
            max_rollouts: Some(100),
            max_lm_calls: Some(200),
            ..Default::default()
        }
    }

    /// Production configuration (thorough, expensive)
    pub fn production() -> Self {
        Self {
            num_iterations: 20,
            minibatch_size: 15,
            num_trials: 8,
            max_rollouts: Some(1000),
            max_lm_calls: Some(2000),
            use_llm_judge: true,
            prompt_model: Some("claude-sonnet-4-5-20250514".to_string()),
            judge_model: Some("claude-sonnet-4-5-20250514".to_string()),
            ..Default::default()
        }
    }

    /// Set the seed instruction
    pub fn with_seed_instruction(mut self, instruction: impl Into<String>) -> Self {
        self.seed_instruction = instruction.into();
        self
    }

    /// Use LLM-as-judge for evaluation
    pub fn with_llm_judge(mut self, model: impl Into<String>) -> Self {
        self.use_llm_judge = true;
        self.judge_model = Some(model.into());
        self
    }

    /// Set the prompt model for reflection/mutation
    pub fn with_prompt_model(mut self, model: impl Into<String>) -> Self {
        self.prompt_model = Some(model.into());
        self
    }
}
