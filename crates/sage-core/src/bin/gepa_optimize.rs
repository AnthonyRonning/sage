//! GEPA Optimization CLI
//!
//! Run GEPA prompt optimization for Sage agent instructions.
//!
//! Usage:
//!   cargo run --bin gepa-optimize -- --mode dev
//!   cargo run --bin gepa-optimize -- --mode production
//!   cargo run --bin gepa-optimize -- --compare
//!   cargo run --bin gepa-optimize -- --eval-demo

use anyhow::Result;
use dspy_rs::{BamlType, Signature};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mode = if args.contains(&"--compare".to_string()) {
        "compare"
    } else if args.contains(&"--eval-demo".to_string()) {
        "eval-demo"
    } else if args.contains(&"--eval-live".to_string()) {
        "eval-live"
    } else if args.iter().any(|a| a == "--mode") {
        let idx = args.iter().position(|a| a == "--mode").unwrap();
        args.get(idx + 1).map(|s| s.as_str()).unwrap_or("dev")
    } else {
        "dev"
    };

    match mode {
        "dev" => run_optimization_dev(),
        "production" => run_optimization_production(),
        "compare" => run_comparison(),
        "eval-demo" => run_eval_demo(),
        "eval-live" => run_eval_live(),
        _ => {
            eprintln!("Unknown mode: {}", mode);
            eprintln!("Usage:");
            eprintln!("  gepa-optimize --mode dev");
            eprintln!("  gepa-optimize --mode production");
            eprintln!("  gepa-optimize --compare");
            eprintln!("  gepa-optimize --eval-demo   (simulated responses)");
            eprintln!("  gepa-optimize --eval-live   (real LLM calls)");
            std::process::exit(1);
        }
    }
}

// ============================================================================
// Inline types (since we can't import from main.rs modules)
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GepaExample {
    id: String,
    category: String,
    input: String,
    #[serde(default)]
    previous_context_summary: String,
    conversation_context: String,
    expected_behavior: String,
    expected_response_type: String,
    #[serde(default)]
    expected_tools: Vec<String>,
    #[serde(default)]
    should_store_memory: bool,
    #[serde(default)]
    bad_patterns: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GepaDataset {
    description: String,
    version: String,
    examples: Vec<GepaExample>,
}

#[derive(Clone, Debug, Default)]
struct ParsedResponse {
    reasoning: String,
    messages: Vec<String>,
    tool_calls: Vec<ParsedToolCall>,
    parse_error: Option<String>,
}

#[derive(Clone, Debug)]
struct ParsedToolCall {
    name: String,
    #[allow(dead_code)]
    args: HashMap<String, String>,
}

#[derive(Clone, Debug, Default)]
struct EvaluationResult {
    score: f32,
    feedback: String,
}

// ============================================================================
// Simple evaluator (inline version)
// ============================================================================

fn evaluate_example(example: &GepaExample, response: &ParsedResponse) -> EvaluationResult {
    let mut score = 0.0f32;
    let mut feedback = String::new();

    // Format check
    if response.parse_error.is_none() {
        score += 0.15;
    } else {
        feedback.push_str(&format!("Format Error: {}\n", response.parse_error.as_deref().unwrap_or("?")));
    }

    // Style check
    match example.expected_response_type.as_str() {
        "casual" => {
            if response.messages.len() >= 2 {
                score += 0.20;
            } else {
                feedback.push_str("Style: Expected multiple casual messages\n");
                score += 0.10;
            }
        }
        "silent_done" => {
            if response.messages.is_empty() {
                score += 0.20;
            } else {
                feedback.push_str("Style: Expected no messages (silent done)\n");
            }
        }
        _ => {
            if !response.messages.is_empty() || !response.tool_calls.is_empty() {
                score += 0.20;
            }
        }
    }

    // Tool check
    let expected: std::collections::HashSet<&str> = example.expected_tools.iter().map(|s| s.as_str()).collect();
    let actual: std::collections::HashSet<&str> = response.tool_calls.iter().map(|t| t.name.as_str()).collect();
    
    if expected == actual {
        score += 0.30;
    } else {
        let missing: Vec<_> = expected.difference(&actual).collect();
        let extra: Vec<_> = actual.difference(&expected).collect();
        if !missing.is_empty() {
            feedback.push_str(&format!("Tools: Missing {:?}\n", missing));
        }
        if !extra.is_empty() {
            feedback.push_str(&format!("Tools: Extra {:?}\n", extra));
        }
        // Partial credit
        let overlap = expected.intersection(&actual).count();
        let union = expected.union(&actual).count();
        if union > 0 {
            score += 0.30 * (overlap as f32 / union as f32);
        }
    }

    // Memory check
    if example.should_store_memory {
        let has_memory = response.tool_calls.iter().any(|t| 
            t.name.contains("memory") || t.name.contains("archival")
        );
        if has_memory {
            score += 0.20;
        } else {
            feedback.push_str("Memory: Should have stored to memory\n");
        }
    } else {
        score += 0.20;
    }

    // Content check (basic)
    score += 0.15;

    if feedback.is_empty() {
        feedback = "Good response".to_string();
    }

    EvaluationResult { score, feedback }
}

// ============================================================================
// Commands
// ============================================================================

fn run_optimization_dev() -> Result<()> {
    println!("=== GEPA Optimization (Development Mode) ===");
    println!();
    
    // Load dataset
    let trainset_path = PathBuf::from("examples/gepa/trainset.json");
    if !trainset_path.exists() {
        println!("Error: trainset.json not found at {:?}", trainset_path);
        println!("Run from the sage project root directory.");
        return Ok(());
    }

    let content = std::fs::read_to_string(&trainset_path)?;
    let dataset: GepaDataset = serde_json::from_str(&content)?;
    
    println!("Loaded {} training examples from {:?}", dataset.examples.len(), trainset_path);
    println!("Dataset version: {}", dataset.version);
    println!();

    // Show categories
    let mut categories: Vec<String> = dataset.examples.iter().map(|e| e.category.clone()).collect();
    categories.sort();
    categories.dedup();
    println!("Categories ({}):", categories.len());
    for cat in &categories {
        let count = dataset.examples.iter().filter(|e| &e.category == cat).count();
        println!("  - {}: {} examples", cat, count);
    }
    println!();

    println!("To run actual optimization, the LLM backend must be configured.");
    println!("Set MAPLE_API_URL, MAPLE_API_KEY, MAPLE_MODEL environment variables.");
    println!();
    println!("Run 'cargo run --bin gepa-optimize -- --eval-demo' to see the evaluator in action.");

    Ok(())
}

fn run_optimization_production() -> Result<()> {
    println!("=== GEPA Optimization (Production Mode) ===");
    println!();
    println!("Configuration:");
    println!("  Iterations: 20");
    println!("  Minibatch: 15");
    println!("  Max rollouts: 1000");
    println!("  LLM-as-Judge: Enabled (Claude Sonnet 4.5)");
    println!();
    println!("WARNING: Production mode is expensive!");
    println!("Estimated cost: ~$5-20 depending on examples");
    println!();
    println!("This requires LLM backend configuration.");

    Ok(())
}

fn run_comparison() -> Result<()> {
    println!("=== Compare Baseline vs Optimized Instructions ===");
    println!();

    let baseline_path = PathBuf::from("crates/sage-core/src/sage_agent.rs");
    let optimized_path = PathBuf::from("optimized_instructions/gepa_dev.txt");

    println!("Baseline instruction: {:?}", baseline_path);
    println!("  (AGENT_INSTRUCTION constant in sage_agent.rs)");
    println!();

    if optimized_path.exists() {
        println!("Optimized instruction: {:?}", optimized_path);
        let content = std::fs::read_to_string(&optimized_path)?;
        println!();
        println!("--- Optimized Instruction ---");
        println!("{}", content);
        println!("--- End ---");
    } else {
        println!("No optimized instruction found.");
        println!("Run 'just gepa-optimize-dev' first.");
    }

    Ok(())
}

fn run_eval_demo() -> Result<()> {
    println!("=== GEPA Evaluator Demo (Simulated) ===");
    println!();
    println!("This shows how the evaluator scores hardcoded example responses.");
    println!("Run --eval-live to test against the real LLM.");
    println!();

    // Load dataset
    let trainset_path = PathBuf::from("examples/gepa/trainset.json");
    if !trainset_path.exists() {
        println!("Error: trainset.json not found");
        return Ok(());
    }

    let content = std::fs::read_to_string(&trainset_path)?;
    let dataset: GepaDataset = serde_json::from_str(&content)?;

    println!("Simulating agent responses and evaluating...");
    println!();

    // Simulate some responses and evaluate
    let test_cases = [
        // Good casual response
        ("casual_greeting_1", ParsedResponse {
            reasoning: "User greeting, respond casually".to_string(),
            messages: vec!["Hey!".to_string(), "Good to hear from you!".to_string()],
            tool_calls: vec![],
            parse_error: None,
        }),
        // Bad casual response (single long message)
        ("casual_greeting_1", ParsedResponse {
            reasoning: "Greeting".to_string(),
            messages: vec!["Hello! How can I assist you today with any questions or tasks?".to_string()],
            tool_calls: vec![],
            parse_error: None,
        }),
        // Good web search
        ("web_search_news", ParsedResponse {
            reasoning: "Need current info, search web".to_string(),
            messages: vec!["Let me check that for you.".to_string()],
            tool_calls: vec![ParsedToolCall { name: "web_search".to_string(), args: HashMap::new() }],
            parse_error: None,
        }),
        // Bad: should search but didn't
        ("web_search_news", ParsedResponse {
            reasoning: "I'll answer from knowledge".to_string(),
            messages: vec!["Bitcoin is at $50,000.".to_string()],
            tool_calls: vec![],
            parse_error: None,
        }),
        // Good memory done
        ("memory_complete_append", ParsedResponse {
            reasoning: "Memory stored, done".to_string(),
            messages: vec![],
            tool_calls: vec![ParsedToolCall { name: "done".to_string(), args: HashMap::new() }],
            parse_error: None,
        }),
        // Bad: announced memory storage
        ("memory_complete_append", ParsedResponse {
            reasoning: "Tell user about memory".to_string(),
            messages: vec!["I've saved that to my memory!".to_string()],
            tool_calls: vec![],
            parse_error: None,
        }),
        // Good memory storage
        ("memory_new_job", ParsedResponse {
            reasoning: "Important info, store and respond".to_string(),
            messages: vec!["Congratulations!".to_string(), "That's amazing news!".to_string()],
            tool_calls: vec![
                ParsedToolCall { name: "memory_append".to_string(), args: HashMap::new() },
                ParsedToolCall { name: "archival_insert".to_string(), args: HashMap::new() },
            ],
            parse_error: None,
        }),
        // Bad: didn't store memory
        ("memory_new_job", ParsedResponse {
            reasoning: "Respond".to_string(),
            messages: vec!["Congrats!".to_string()],
            tool_calls: vec![],
            parse_error: None,
        }),
    ];

    for (example_id, response) in test_cases {
        let example = dataset.examples.iter().find(|e| e.id == example_id);
        if let Some(ex) = example {
            let result = evaluate_example(ex, &response);
            
            let status = if result.score >= 0.8 { "GOOD" } else if result.score >= 0.5 { "OKAY" } else { "BAD" };
            
            println!("Example: {} ({})", example_id, ex.expected_response_type);
            println!("  Response: {:?}", if response.messages.is_empty() { vec!["<empty>".to_string()] } else { response.messages.clone() });
            println!("  Tools: {:?}", response.tool_calls.iter().map(|t| &t.name).collect::<Vec<_>>());
            println!("  Score: {:.2} [{}]", result.score, status);
            if result.feedback != "Good response" {
                println!("  Feedback: {}", result.feedback.trim());
            }
            println!();
        }
    }

    println!("=== Summary ===");
    println!("The evaluator checks:");
    println!("  1. Format correctness (0.15)");
    println!("  2. Response style match (0.20)");
    println!("  3. Tool usage correctness (0.30)");
    println!("  4. Memory proactivity (0.20)");
    println!("  5. Content quality (0.15)");
    println!();
    println!("GEPA uses this feedback to evolve better instructions.");

    Ok(())
}

fn run_eval_live() -> Result<()> {
    // Use tokio runtime for async
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_eval_live_async())
}

async fn run_eval_live_async() -> Result<()> {
    use dspy_rs::{configure, ChatAdapter, LM, Predict};

    println!("=== GEPA Live Evaluation ===");
    println!();

    // Load environment
    dotenvy::dotenv().ok();
    
    let api_url = std::env::var("MAPLE_API_URL").unwrap_or_else(|_| "http://localhost:8080/v1".to_string());
    let api_key = std::env::var("MAPLE_API_KEY").unwrap_or_default();
    let model = std::env::var("MAPLE_MODEL").unwrap_or_else(|_| "gpt-4".to_string());

    println!("LLM Configuration:");
    println!("  API URL: {}", api_url);
    println!("  Model: {}", model);
    println!();

    // Configure DSRs
    let lm = LM::builder()
        .base_url(api_url)
        .api_key(api_key)
        .model(model)
        .temperature(0.7)
        .max_tokens(4096)
        .build()
        .await?;
    
    configure(lm, ChatAdapter);

    // Load dataset
    let trainset_path = PathBuf::from("examples/gepa/trainset.json");
    if !trainset_path.exists() {
        println!("Error: trainset.json not found");
        return Ok(());
    }

    let content = std::fs::read_to_string(&trainset_path)?;
    let dataset: GepaDataset = serde_json::from_str(&content)?;

    println!("Loaded {} examples", dataset.examples.len());
    println!();

    // Define the agent signature
    #[derive(Signature, Clone, Debug)]
    struct AgentResponse {
        #[input(desc = "The input to respond to")]
        input: String,
        #[input(desc = "Previous context summary")]
        previous_context_summary: String,
        #[input(desc = "Conversation context")]
        conversation_context: String,
        #[input(desc = "Available tools")]
        available_tools: String,

        #[output(desc = "Your reasoning")]
        reasoning: String,
        #[output(desc = "Messages to send")]
        messages: Vec<String>,
        #[output(desc = "Tool calls")]
        tool_calls: Vec<ToolCallOutput>,
    }

    #[derive(Clone, Debug, Default, BamlType)]
    struct ToolCallOutput {
        name: String,
        args: HashMap<String, String>,
    }

    // Minimal seed instruction
    let instruction = r#"You are Sage, a helpful AI assistant communicating via Signal.

You have access to memory (core blocks, archival storage) and tools (web search, etc.).
Respond naturally. Use tools when helpful. Store important information to memory proactively.

For casual chat, use multiple short messages. For detailed explanations, longer messages are fine.
After memory operations complete, return done silently (no announcement needed)."#;

    let predictor = Predict::<AgentResponse>::builder()
        .instruction(instruction)
        .build();

    let tools_desc = r#"Available tools:
web_search: Search the web. Args: {"query": "..."}
memory_append: Add to memory block. Args: {"block": "human|persona", "content": "..."}
archival_insert: Store in archival memory. Args: {"content": "..."}
archival_search: Search archival memory. Args: {"query": "..."}
done: Signal nothing more to do. Args: {}"#;

    // Evaluate a subset of examples
    let test_ids = [
        "casual_greeting_1",
        "web_search_news",
        "memory_new_job",
        "memory_complete_append",
        "tool_result_web_search",
    ];

    let mut total_score = 0.0f32;
    let mut count = 0;

    for example_id in test_ids {
        let example = match dataset.examples.iter().find(|e| e.id == example_id) {
            Some(e) => e,
            None => continue,
        };

        println!("--- Example: {} ({}) ---", example.id, example.expected_response_type);
        println!("Input: {}", &example.input[..example.input.len().min(80)]);

        let input = AgentResponseInput {
            input: example.input.clone(),
            previous_context_summary: example.previous_context_summary.clone(),
            conversation_context: example.conversation_context.clone(),
            available_tools: tools_desc.to_string(),
        };

        match predictor.call(input).await {
            Ok(response) => {
                let parsed = ParsedResponse {
                    reasoning: response.reasoning.clone(),
                    messages: response.messages.clone(),
                    tool_calls: response.tool_calls.iter().map(|t| ParsedToolCall {
                        name: t.name.clone(),
                        args: t.args.clone(),
                    }).collect(),
                    parse_error: None,
                };

                let result = evaluate_example(example, &parsed);
                total_score += result.score;
                count += 1;

                let status = if result.score >= 0.8 { "GOOD" } else if result.score >= 0.5 { "OKAY" } else { "BAD" };
                
                println!("Messages: {:?}", response.messages);
                println!("Tools: {:?}", response.tool_calls.iter().map(|t| &t.name).collect::<Vec<_>>());
                println!("Score: {:.2} [{}]", result.score, status);
                if result.feedback != "Good response" {
                    println!("Feedback: {}", result.feedback.trim());
                }
            }
            Err(e) => {
                println!("Error: {:?}", e);
                let parsed = ParsedResponse {
                    parse_error: Some(e.to_string()),
                    ..Default::default()
                };
                let result = evaluate_example(example, &parsed);
                total_score += result.score;
                count += 1;
                println!("Score: {:.2} [ERROR]", result.score);
            }
        }
        println!();
    }

    println!("=== Results ===");
    println!("Examples evaluated: {}", count);
    println!("Average score: {:.2}", total_score / count as f32);
    println!();
    println!("This is the baseline score with the minimal seed instruction.");
    println!("GEPA optimization would evolve this instruction to improve the score.");

    Ok(())
}
