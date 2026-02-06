//! GEPA Optimization CLI for Sage
//!
//! Implements GEPA (Genetic-Pareto) reflective prompt optimization
//! following the official DSRs patterns.
//!
//! Usage:
//!   cargo run --bin gepa-optimize -- --eval         (evaluate baseline)
//!   cargo run --bin gepa-optimize -- --optimize     (run GEPA optimization)

use anyhow::Result;
use dspy_rs::{configure, ChatAdapter, FeedbackMetric, Predict, Signature, LM};
use sage_core::{
    canonical_tool_descriptions, AgentResponse, AgentResponseInput, AGENT_INSTRUCTION,
};
use std::collections::HashMap;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.contains(&"--optimize".to_string()) {
        run_optimization()
    } else {
        run_evaluation()
    }
}

// ============================================================================
// Evaluator with rich feedback (DSRs FeedbackEvaluator pattern)
// ============================================================================

fn evaluate_with_feedback(
    example: &TrainingExample,
    messages: &[String],
    tool_names: &[String],
) -> FeedbackMetric {
    let mut score = 0.0f32;
    let mut feedback = String::new();

    // Check 1: First-time user should ask for name (0.35)
    if example.is_first_time_user && example.human_block.is_empty() {
        let asks_name = messages.iter().any(|m| {
            let lower = m.to_lowercase();
            lower.contains("name") || lower.contains("call you") || lower.contains("who are you")
        });
        if asks_name {
            score += 0.35;
            feedback.push_str("✓ Asked for user's name (first-time user)\n");
        } else {
            feedback.push_str("✗ Did NOT ask for name (first-time user with empty human_block)\n");
        }
    } else {
        score += 0.35; // N/A
    }

    // Check 2: Message style (0.25)
    if example.expected_behavior.contains("casual")
        || example.expected_behavior.contains("multiple")
    {
        if messages.len() >= 2 {
            score += 0.25;
            feedback.push_str(&format!(
                "✓ Multiple messages ({} messages)\n",
                messages.len()
            ));
        } else {
            feedback.push_str(&format!(
                "✗ Expected multiple casual messages, got {}\n",
                messages.len()
            ));
        }
    } else if example.expected_behavior.contains("silent")
        || example.expected_behavior.contains("done")
    {
        if messages.is_empty() && tool_names.contains(&"done".to_string()) {
            score += 0.25;
            feedback.push_str("✓ Silent done (no messages, done tool)\n");
        } else {
            feedback.push_str("✗ Expected silent done\n");
        }
    } else {
        score += 0.25;
    }

    // Check 3: Expected tools (0.30)
    if example.expected_behavior.contains("memory_append") {
        if tool_names.iter().any(|t| t.contains("memory")) {
            score += 0.30;
            feedback.push_str("✓ Used memory tool\n");
        } else {
            feedback.push_str("✗ Expected memory tool usage\n");
        }
    } else if example.expected_behavior.contains("archival") {
        if tool_names.iter().any(|t| t.contains("archival")) {
            score += 0.30;
            feedback.push_str("✓ Used archival tool\n");
        } else {
            feedback.push_str("✗ Expected archival tool usage\n");
        }
    } else if example.expected_behavior.contains("web_search") {
        if tool_names.contains(&"web_search".to_string()) {
            score += 0.30;
            feedback.push_str("✓ Used web_search\n");
        } else {
            feedback.push_str("✗ Expected web_search\n");
        }
    } else {
        score += 0.30;
    }

    // Check 4: Parse success (0.10) - if we got here, parsing succeeded
    score += 0.10;
    feedback.push_str("✓ Response parsed successfully\n");

    feedback.push_str(&format!("\nExpected: {}\n", example.expected_behavior));
    feedback.push_str(&format!("Messages: {:?}\n", messages));
    feedback.push_str(&format!("Tools: {:?}\n", tool_names));

    FeedbackMetric::new(score, feedback)
}

// ============================================================================
// Training Data
// ============================================================================

#[derive(Clone)]
struct TrainingExample {
    input: String,
    current_time: String,
    persona_block: String,
    human_block: String,
    memory_metadata: String,
    previous_context_summary: String,
    recent_conversation: String,
    is_first_time_user: bool,
    expected_behavior: String,
}

fn load_trainset() -> Vec<TrainingExample> {
    // Load from JSON file
    let json_path = std::path::Path::new("examples/gepa/trainset.json");
    if json_path.exists() {
        let content = std::fs::read_to_string(json_path).expect("Failed to read trainset.json");
        let json: serde_json::Value =
            serde_json::from_str(&content).expect("Failed to parse trainset.json");

        let examples = json["examples"].as_array().expect("No examples array");
        return examples
            .iter()
            .map(|e| TrainingExample {
                input: e["input"].as_str().unwrap_or("").to_string(),
                current_time: e["current_time"].as_str().unwrap_or("").to_string(),
                persona_block: e["persona_block"].as_str().unwrap_or("").to_string(),
                human_block: e["human_block"].as_str().unwrap_or("").to_string(),
                memory_metadata: e["memory_metadata"].as_str().unwrap_or("").to_string(),
                previous_context_summary: e["previous_context_summary"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                recent_conversation: e["recent_conversation"].as_str().unwrap_or("").to_string(),
                is_first_time_user: e["is_first_time_user"].as_bool().unwrap_or(false),
                expected_behavior: e["expected_behavior"].as_str().unwrap_or("").to_string(),
            })
            .collect();
    }

    // Fallback if file doesn't exist
    eprintln!("Warning: examples/gepa/trainset.json not found, using empty trainset");
    vec![]
}

// ============================================================================
// Main Entry Points
// ============================================================================

fn run_evaluation() -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_evaluation_async())
}

async fn run_evaluation_async() -> Result<()> {
    println!("=== GEPA Baseline Evaluation ===\n");

    dotenvy::dotenv().ok();

    let api_url =
        std::env::var("MAPLE_API_URL").unwrap_or_else(|_| "http://localhost:8089/v1".into());
    let api_key = std::env::var("MAPLE_API_KEY").unwrap_or_else(|_| "test".into());
    let model = std::env::var("MAPLE_MODEL").unwrap_or_else(|_| "gpt-4".into());

    println!("Program LM: {} @ {}\n", model, api_url);

    let lm = LM::builder()
        .base_url(api_url)
        .api_key(api_key)
        .model(model)
        .temperature(0.7)
        .max_tokens(4096)
        .build()
        .await?;

    configure(lm, ChatAdapter);

    let instruction = load_instruction();
    println!("Instruction length: {} chars\n", instruction.len());

    let predictor = Predict::<AgentResponse>::builder()
        .instruction(&instruction)
        .build();

    let trainset = load_trainset();
    println!("Training examples: {}\n", trainset.len());

    let mut total_score = 0.0f32;

    for example in &trainset {
        let input = AgentResponseInput {
            input: example.input.clone(),
            current_time: example.current_time.clone(),
            persona_block: example.persona_block.clone(),
            human_block: example.human_block.clone(),
            memory_metadata: example.memory_metadata.clone(),
            previous_context_summary: example.previous_context_summary.clone(),
            recent_conversation: example.recent_conversation.clone(),
            available_tools: canonical_tool_descriptions(),
            is_first_time_user: example.is_first_time_user,
        };

        let input_short = &example.input[..example.input.len().min(40)];

        match predictor.call(input).await {
            Ok(response) => {
                let tool_names: Vec<String> =
                    response.tool_calls.iter().map(|t| t.name.clone()).collect();
                let feedback = evaluate_with_feedback(example, &response.messages, &tool_names);
                total_score += feedback.score;

                let status = if feedback.score >= 0.8 {
                    "✓"
                } else if feedback.score >= 0.5 {
                    "~"
                } else {
                    "✗"
                };
                println!("{} [{:.2}] {}", status, feedback.score, input_short);
            }
            Err(e) => {
                println!("✗ [0.00] {} - Error: {:?}", input_short, e);
            }
        }
    }

    println!("\n=== Results ===");
    println!("Average score: {:.3}", total_score / trainset.len() as f32);
    println!("\nRun with --optimize to run GEPA optimization");

    Ok(())
}

fn run_optimization() -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_optimization_async())
}

// ============================================================================
// GEPA Reflection Signatures (DSRs pattern - used by Claude judge)
// ============================================================================

/// Signature for analyzing execution traces
#[derive(Signature, Clone, Debug)]
struct ReflectOnTraces {
    #[input(desc = "The current instruction being optimized")]
    current_instruction: String,

    #[input(desc = "Execution traces with inputs, outputs, and feedback for failed examples")]
    failed_traces: String,

    #[input(desc = "Description of what the agent should accomplish")]
    task_description: String,

    #[output(desc = "Analysis of specific weaknesses and concrete improvement suggestions")]
    reflection: String,
}

/// Signature for proposing improved instruction
#[derive(Signature, Clone, Debug)]
struct ProposeInstruction {
    #[input(desc = "The current instruction")]
    current_instruction: String,

    #[input(desc = "Analysis of weaknesses and improvement suggestions")]
    reflection: String,

    #[output(desc = "The complete improved instruction that addresses the identified issues")]
    improved_instruction: String,
}

// ============================================================================
// GEPA Candidate tracking
// ============================================================================

#[derive(Clone, Debug)]
struct GEPACandidate {
    instruction: String,
    scores: HashMap<usize, f32>,
    #[allow(dead_code)]
    generation: usize,
}

impl GEPACandidate {
    fn average_score(&self) -> f32 {
        if self.scores.is_empty() {
            return 0.0;
        }
        self.scores.values().sum::<f32>() / self.scores.len() as f32
    }
}

// ============================================================================
// Execution Trace for reflection
// ============================================================================

#[derive(Clone, Debug)]
struct ExecutionTrace {
    example_idx: usize,
    input: String,
    expected_behavior: String,
    actual_messages: Vec<String>,
    actual_tools: Vec<String>,
    score: f32,
    feedback: String,
}

impl ExecutionTrace {
    fn format_for_reflection(&self) -> String {
        format!(
            "Example {}: Input: \"{}\"\n\
             Expected: {}\n\
             Actual messages: {:?}\n\
             Actual tools: {:?}\n\
             Score: {:.2}\n\
             Feedback: {}",
            self.example_idx,
            &self.input[..self.input.len().min(60)],
            self.expected_behavior,
            self.actual_messages,
            self.actual_tools,
            self.score,
            self.feedback
        )
    }
}

async fn run_optimization_async() -> Result<()> {
    println!("=== GEPA Optimization ===\n");

    dotenvy::dotenv().ok();

    // Configure program LM (Kimi - the model being optimized)
    let api_url =
        std::env::var("MAPLE_API_URL").unwrap_or_else(|_| "http://localhost:8089/v1".into());
    let api_key = std::env::var("MAPLE_API_KEY").unwrap_or_else(|_| "test".into());
    let model = std::env::var("MAPLE_MODEL").unwrap_or_else(|_| "gpt-4".into());

    println!("Program LM: {} @ {}", model, api_url);

    let program_lm = LM::builder()
        .base_url(api_url.clone())
        .api_key(api_key.clone())
        .model(model.clone())
        .temperature(0.7)
        .max_tokens(4096)
        .build()
        .await?;

    // Configure judge LM (Claude via Anthropic API - for reflection/mutation)
    let judge_api_key =
        std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY must be set for GEPA judge");
    let judge_model = std::env::var("GEPA_JUDGE_MODEL")
        .unwrap_or_else(|_| "anthropic:claude-sonnet-4-5-20250929".into());

    println!("Judge LM: {} (via Anthropic API)\n", judge_model);

    let judge_lm = LM::builder()
        .api_key(judge_api_key)
        .model(judge_model)
        .temperature(0.9)
        .max_tokens(8192)
        .build()
        .await?;

    // Load training data
    let trainset = load_trainset();
    println!("Training examples: {}", trainset.len());

    // GEPA parameters
    const MAX_ITERATIONS: usize = 5;
    const TASK_DESCRIPTION: &str = "Sage is an AI assistant on Signal. \
        For FIRST-TIME USERS (is_first_time_user=true AND empty human_block), it MUST ask for their name. \
        For casual chat, use 2-4 short messages. \
        For major life events, use BOTH memory_append AND archival_insert. \
        After memory tool results, return done silently (no message).";

    // Initialize with current instruction
    let mut best_candidate = GEPACandidate {
        instruction: load_instruction(),
        scores: HashMap::new(),
        generation: 0,
    };

    let mut evolution_history: Vec<(usize, f32)> = Vec::new();

    // Evaluate baseline
    println!("\n============================================================");
    println!("Generation 0: Baseline");
    println!("============================================================\n");

    configure(program_lm.clone(), ChatAdapter);
    let (baseline_scores, baseline_traces) =
        evaluate_instruction(&best_candidate.instruction, &trainset).await;
    best_candidate.scores = baseline_scores;
    let baseline_score = best_candidate.average_score();
    evolution_history.push((0, baseline_score));

    println!("Baseline score: {:.3}", baseline_score);
    print_scores(&best_candidate.scores, &trainset);

    // Main GEPA loop
    for generation in 1..=MAX_ITERATIONS {
        println!("\n============================================================");
        println!("Generation {}", generation);
        println!("============================================================\n");

        // Stop if perfect
        if best_candidate.average_score() >= 0.99 {
            println!("Near-perfect score. Stopping.");
            break;
        }

        // Get failed traces
        let failed_traces: Vec<_> = baseline_traces.iter().filter(|t| t.score < 0.95).collect();

        if failed_traces.is_empty() {
            println!("No failures to address. Stopping.");
            break;
        }

        println!("Failures to address: {}", failed_traces.len());
        for t in &failed_traces {
            println!(
                "  - Example {} ({:.2}): {}",
                t.example_idx,
                t.score,
                &t.input[..t.input.len().min(30)]
            );
        }

        // GEPA Reflection with Claude
        println!("\nReflecting on failures (using judge LM)...");
        configure(judge_lm.clone(), ChatAdapter);

        let traces_text = failed_traces
            .iter()
            .map(|t| t.format_for_reflection())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        // Step 1: Reflect on traces
        let reflect_predictor = Predict::<ReflectOnTraces>::builder()
            .instruction(
                "You are an expert prompt engineer analyzing why an AI assistant failed certain test cases. \
                 Identify specific patterns in the failures and suggest concrete fixes. \
                 Be specific - point to exact phrases that should be added or changed."
            )
            .build();

        let reflection = match reflect_predictor
            .call(ReflectOnTracesInput {
                current_instruction: best_candidate.instruction.clone(),
                failed_traces: traces_text.clone(),
                task_description: TASK_DESCRIPTION.to_string(),
            })
            .await
        {
            Ok(r) => {
                println!("\n--- Reflection ---");
                println!("{}", &r.reflection[..r.reflection.len().min(500)]);
                if r.reflection.len() > 500 {
                    println!("...");
                }
                println!("---\n");
                r.reflection
            }
            Err(e) => {
                println!("Reflection failed: {:?}", e);
                continue;
            }
        };

        // Step 2: Propose improved instruction
        let propose_predictor = Predict::<ProposeInstruction>::builder()
            .instruction(
                "You are an expert prompt engineer. Given the reflection on failures, \
                 output an IMPROVED version of the instruction that fixes the issues. \
                 Output ONLY the complete instruction text, starting with 'You are Sage'. \
                 Keep the same structure but add/modify rules to fix the failures.",
            )
            .build();

        let improved_instruction = match propose_predictor
            .call(ProposeInstructionInput {
                current_instruction: best_candidate.instruction.clone(),
                reflection,
            })
            .await
        {
            Ok(r) => r.improved_instruction,
            Err(e) => {
                println!("Proposal failed: {:?}", e);
                continue;
            }
        };

        // Evaluate new instruction
        println!("Evaluating improved instruction...");
        configure(program_lm.clone(), ChatAdapter);
        let (new_scores, _new_traces) =
            evaluate_instruction(&improved_instruction, &trainset).await;

        let new_candidate = GEPACandidate {
            instruction: improved_instruction,
            scores: new_scores,
            generation,
        };
        let new_score = new_candidate.average_score();

        println!(
            "\nNew score: {:.3} (was {:.3})",
            new_score,
            best_candidate.average_score()
        );
        print_score_comparison(&best_candidate.scores, &new_candidate.scores, &trainset);

        // Update if improved
        if new_score > best_candidate.average_score() {
            println!("\n*** Improvement! Updating best candidate. ***");
            best_candidate = new_candidate;
            evolution_history.push((generation, new_score));
        } else {
            println!("\nNo improvement. Keeping previous best.");
            evolution_history.push((generation, best_candidate.average_score()));
        }
    }

    // Final results
    println!("\n============================================================");
    println!("OPTIMIZATION COMPLETE");
    println!("============================================================");

    println!("\nEvolution:");
    for (gen, score) in &evolution_history {
        println!("  Gen {}: {:.3}", gen, score);
    }

    let improvement = best_candidate.average_score() - baseline_score;
    println!(
        "\nFinal: {:.3} (improvement: {:+.3})",
        best_candidate.average_score(),
        improvement
    );

    // Save optimized instruction
    let output_path = PathBuf::from("optimized_instructions/latest.txt");
    std::fs::create_dir_all("optimized_instructions")?;
    std::fs::write(&output_path, &best_candidate.instruction)?;
    println!("\nSaved to: {}", output_path.display());

    // Also update AGENT_INSTRUCTION in sage_agent.rs if score improved significantly
    if improvement > 0.05 {
        println!("\n*** Significant improvement! Consider updating AGENT_INSTRUCTION in sage_agent.rs ***");
    }

    println!("\n=== Optimized Instruction ===\n");
    println!("{}", best_candidate.instruction);

    Ok(())
}

async fn evaluate_instruction(
    instruction: &str,
    trainset: &[TrainingExample],
) -> (HashMap<usize, f32>, Vec<ExecutionTrace>) {
    let predictor = Predict::<AgentResponse>::builder()
        .instruction(instruction)
        .build();

    let mut scores = HashMap::new();
    let mut traces = Vec::new();

    for (idx, example) in trainset.iter().enumerate() {
        let input = AgentResponseInput {
            input: example.input.clone(),
            current_time: example.current_time.clone(),
            persona_block: example.persona_block.clone(),
            human_block: example.human_block.clone(),
            memory_metadata: example.memory_metadata.clone(),
            previous_context_summary: example.previous_context_summary.clone(),
            recent_conversation: example.recent_conversation.clone(),
            available_tools: canonical_tool_descriptions(),
            is_first_time_user: example.is_first_time_user,
        };

        match predictor.call(input).await {
            Ok(response) => {
                let tool_names: Vec<String> =
                    response.tool_calls.iter().map(|t| t.name.clone()).collect();
                let feedback = evaluate_with_feedback(example, &response.messages, &tool_names);

                scores.insert(idx, feedback.score);
                traces.push(ExecutionTrace {
                    example_idx: idx,
                    input: example.input.clone(),
                    expected_behavior: example.expected_behavior.clone(),
                    actual_messages: response.messages,
                    actual_tools: tool_names,
                    score: feedback.score,
                    feedback: feedback.feedback.clone(),
                });
            }
            Err(e) => {
                scores.insert(idx, 0.0);
                traces.push(ExecutionTrace {
                    example_idx: idx,
                    input: example.input.clone(),
                    expected_behavior: example.expected_behavior.clone(),
                    actual_messages: vec![],
                    actual_tools: vec![],
                    score: 0.0,
                    feedback: format!("Error: {:?}", e),
                });
            }
        }
    }

    (scores, traces)
}

fn print_scores(scores: &HashMap<usize, f32>, trainset: &[TrainingExample]) {
    for (idx, example) in trainset.iter().enumerate() {
        let score = scores.get(&idx).unwrap_or(&0.0);
        let status = if *score >= 0.95 {
            "✓"
        } else if *score >= 0.7 {
            "~"
        } else {
            "✗"
        };
        let input_short = &example.input[..example.input.len().min(35)];
        println!("  {} [{:.2}] {}", status, score, input_short);
    }
}

fn print_score_comparison(
    old: &HashMap<usize, f32>,
    new: &HashMap<usize, f32>,
    trainset: &[TrainingExample],
) {
    for (idx, example) in trainset.iter().enumerate() {
        let old_score = old.get(&idx).unwrap_or(&0.0);
        let new_score = new.get(&idx).unwrap_or(&0.0);
        let delta = new_score - old_score;
        let arrow = if delta > 0.01 {
            "↑"
        } else if delta < -0.01 {
            "↓"
        } else {
            "="
        };
        let status = if *new_score >= 0.95 {
            "✓"
        } else if *new_score >= 0.7 {
            "~"
        } else {
            "✗"
        };
        let input_short = &example.input[..example.input.len().min(30)];
        println!("  {} [{:.2}] {} {}", status, new_score, input_short, arrow);
    }
}

fn load_instruction() -> String {
    let optimized_path = PathBuf::from("optimized_instructions/latest.txt");
    if optimized_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&optimized_path) {
            return content;
        }
    }
    AGENT_INSTRUCTION.to_string()
}
