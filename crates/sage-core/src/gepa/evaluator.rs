//! FeedbackEvaluator implementation for GEPA
//!
//! Provides both rule-based and LLM-as-Judge evaluation approaches.

use super::GepaExample;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Result of evaluating a single agent response
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvaluationResult {
    /// Overall score (0.0 - 1.0)
    pub score: f32,
    /// Rich textual feedback for GEPA reflection
    pub feedback: String,
    /// Component scores for debugging
    pub component_scores: ComponentScores,
}

/// Breakdown of score components
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ComponentScores {
    /// Format correctness (proper fields, no errors)
    pub format: f32,
    /// Response style match (casual vs detailed)
    pub style: f32,
    /// Tool usage correctness
    pub tools: f32,
    /// Memory proactivity
    pub memory: f32,
    /// Content quality (not repeating, natural)
    pub content: f32,
}

/// Parsed agent response for evaluation
#[derive(Clone, Debug, Default)]
pub struct ParsedResponse {
    pub reasoning: String,
    pub messages: Vec<String>,
    pub tool_calls: Vec<ParsedToolCall>,
    pub parse_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ParsedToolCall {
    pub name: String,
    pub args: std::collections::HashMap<String, String>,
}

/// Rule-based feedback evaluator
///
/// Faster and cheaper than LLM-as-judge, but less nuanced.
pub fn evaluate_rule_based(example: &GepaExample, response: &ParsedResponse) -> EvaluationResult {
    let mut feedback = String::new();
    let mut components = ComponentScores::default();

    // 1. Format correctness (0.15 points)
    if response.parse_error.is_none() {
        components.format = 0.15;
    } else {
        feedback.push_str(&format!(
            "Format Error: {}\n  Issue: Response could not be parsed correctly\n",
            response.parse_error.as_deref().unwrap_or("Unknown error")
        ));
    }

    // 2. Response style match (0.20 points)
    components.style = evaluate_style(example, response, &mut feedback);

    // 3. Tool usage correctness (0.30 points)
    components.tools = evaluate_tools(example, response, &mut feedback);

    // 4. Memory proactivity (0.20 points)
    components.memory = evaluate_memory(example, response, &mut feedback);

    // 5. Content quality (0.15 points)
    components.content = evaluate_content(example, response, &mut feedback);

    let score = components.format
        + components.style
        + components.tools
        + components.memory
        + components.content;

    // Add positive feedback if score is high
    if score >= 0.8 {
        feedback.insert_str(0, "Good response overall.\n");
    }

    EvaluationResult {
        score,
        feedback,
        component_scores: components,
    }
}

fn evaluate_style(example: &GepaExample, response: &ParsedResponse, feedback: &mut String) -> f32 {
    let max_score = 0.20;

    match example.expected_response_type.as_str() {
        "casual" => {
            // Casual: expect multiple short messages
            if response.messages.len() >= 2 {
                let avg_len: usize =
                    response.messages.iter().map(|m| m.len()).sum::<usize>() / response.messages.len();
                if avg_len < 150 {
                    return max_score;
                }
                feedback.push_str(&format!(
                    "Style Mismatch\n  Expected: Multiple short casual messages\n  Got: {} messages with avg length {}\n  Suggestion: For casual chat, use 2-4 short messages\n",
                    response.messages.len(), avg_len
                ));
                return max_score * 0.5;
            }
            feedback.push_str(&format!(
                "Style Mismatch\n  Expected: Multiple short messages for casual chat\n  Got: {} message(s)\n  Suggestion: Split response into 2-4 natural chat-like messages\n",
                response.messages.len()
            ));
            max_score * 0.3
        }
        "detailed" => {
            // Detailed: longer messages OK, but should have content
            if !response.messages.is_empty() {
                let total_len: usize = response.messages.iter().map(|m| m.len()).sum();
                if total_len >= 100 {
                    return max_score;
                }
                feedback.push_str(
                    "Style Mismatch\n  Expected: Detailed explanation\n  Got: Brief response\n  Suggestion: Provide more thorough explanation\n",
                );
                return max_score * 0.5;
            }
            feedback.push_str(
                "Style Mismatch\n  Expected: Detailed response\n  Got: No messages\n",
            );
            0.0
        }
        "tool_use" => {
            // Tool use: should have tools called
            if !response.tool_calls.is_empty()
                && response.tool_calls.iter().all(|t| t.name != "done")
            {
                return max_score;
            }
            // Handled in tool evaluation
            max_score * 0.5
        }
        "silent_done" => {
            // Silent done: no messages, just "done" tool
            if response.messages.is_empty() {
                return max_score;
            }
            feedback.push_str(&format!(
                "Style Mismatch\n  Expected: Silent (no messages, just done)\n  Got: {} message(s)\n  Issue: Memory operations should complete silently\n",
                response.messages.len()
            ));
            0.0
        }
        "summarize_results" => {
            // Should have messages summarizing tool results
            if !response.messages.is_empty() {
                return max_score;
            }
            feedback.push_str(
                "Style Mismatch\n  Expected: Summary of tool results\n  Got: No messages\n  Issue: Should summarize findings for user\n",
            );
            0.0
        }
        "acknowledge_and_store" => {
            // Should have both messages and memory tools
            if !response.messages.is_empty() {
                return max_score;
            }
            feedback.push_str(
                "Style Mismatch\n  Expected: Acknowledgment message\n  Got: No messages\n",
            );
            0.0
        }
        _ => {
            // Default: just check there's some response
            if !response.messages.is_empty() || !response.tool_calls.is_empty() {
                max_score
            } else {
                feedback.push_str("No Response\n  Issue: Agent produced no output\n");
                0.0
            }
        }
    }
}

fn evaluate_tools(example: &GepaExample, response: &ParsedResponse, feedback: &mut String) -> f32 {
    let max_score = 0.30;

    let expected: HashSet<&str> = example.expected_tools.iter().map(|s| s.as_str()).collect();
    let actual: HashSet<&str> = response
        .tool_calls
        .iter()
        .map(|t| t.name.as_str())
        .collect();

    // Check for "done" specifically
    let expects_done = expected.contains("done");
    let has_done = actual.contains("done");

    if expects_done {
        if has_done && response.messages.is_empty() && actual.len() == 1 {
            return max_score;
        }
        if !has_done {
            feedback.push_str(
                "Tool Error\n  Expected: done (silent completion)\n  Got: Other tools or messages\n  Issue: After memory operations, should return done\n",
            );
            return 0.0;
        }
    }

    // Filter out "done" for comparison
    let expected_tools: HashSet<&str> = expected.iter().copied().filter(|t| *t != "done").collect();
    let actual_tools: HashSet<&str> = actual.iter().copied().filter(|t| *t != "done").collect();

    if expected_tools.is_empty() && actual_tools.is_empty() {
        return max_score;
    }

    let missing: Vec<&str> = expected_tools.difference(&actual_tools).copied().collect();
    let extra: Vec<&str> = actual_tools.difference(&expected_tools).copied().collect();

    if missing.is_empty() && extra.is_empty() {
        return max_score;
    }

    let mut score = max_score;

    if !missing.is_empty() {
        feedback.push_str(&format!(
            "Tool Error\n  Expected: {:?}\n  Missing: {:?}\n  Issue: Required tools not called\n",
            expected_tools, missing
        ));
        score -= max_score * 0.5;
    }

    if !extra.is_empty() {
        feedback.push_str(&format!(
            "Tool Warning\n  Unexpected tools: {:?}\n  Issue: Called tools that weren't needed\n",
            extra
        ));
        score -= max_score * 0.2;
    }

    score.max(0.0)
}

fn evaluate_memory(example: &GepaExample, response: &ParsedResponse, feedback: &mut String) -> f32 {
    let max_score = 0.20;

    if !example.should_store_memory {
        return max_score;
    }

    let memory_tools = ["memory_append", "memory_replace", "memory_insert", "archival_insert"];
    let has_memory_op = response
        .tool_calls
        .iter()
        .any(|t| memory_tools.contains(&t.name.as_str()));

    if has_memory_op {
        return max_score;
    }

    feedback.push_str(
        "Memory Error\n  Expected: Memory storage operation\n  Got: No memory tools called\n  Issue: Important information should be proactively stored to memory\n",
    );
    0.0
}

fn evaluate_content(example: &GepaExample, response: &ParsedResponse, feedback: &mut String) -> f32 {
    let max_score: f32 = 0.15;
    let mut score: f32 = max_score;

    // Check for bad patterns
    for bad in &example.bad_patterns {
        let bad_lower = bad.to_lowercase();
        for msg in &response.messages {
            if msg.to_lowercase().contains(&bad_lower) {
                feedback.push_str(&format!(
                    "Content Warning\n  Issue: Response matches bad pattern '{}'\n",
                    bad
                ));
                score -= max_score * 0.3;
            }
        }
    }

    // Check for common issues
    let messages_combined: String = response.messages.join(" ").to_lowercase();

    // Don't announce memory operations
    if messages_combined.contains("i'll remember")
        || messages_combined.contains("i've saved")
        || messages_combined.contains("stored to memory")
        || messages_combined.contains("noted that")
    {
        if example.should_store_memory {
            feedback.push_str(
                "Content Warning\n  Issue: Announced memory operation to user\n  Suggestion: Memory operations should be silent\n",
            );
            score -= max_score * 0.5;
        }
    }

    // Check for robotic phrasing
    let robotic = [
        "how can i assist you today",
        "is there anything else",
        "i'm here to help",
        "let me know if you need",
    ];
    for phrase in robotic {
        if messages_combined.contains(phrase) {
            feedback.push_str(&format!(
                "Content Warning\n  Issue: Robotic/formal phrasing '{}'\n  Suggestion: Be more natural and conversational\n",
                phrase
            ));
            score -= max_score * 0.2;
        }
    }

    score.max(0.0)
}

/// LLM-as-Judge prompt template
pub const JUDGE_PROMPT: &str = r#"You are evaluating an AI assistant's response. Score from 0.0-1.0 with detailed feedback.

**Input:** {input}
**Expected Behavior:** {expected_behavior}
**Expected Response Type:** {expected_response_type}
**Expected Tools:** {expected_tools}
**Should Store Memory:** {should_store_memory}

**Actual Response:**
- Reasoning: {reasoning}
- Messages: {messages}
- Tool Calls: {tool_calls}

**Evaluation Criteria:**
1. Response appropriateness (casual vs detailed, multiple messages vs single)
2. Correct tool usage (right tools called at the right time)
3. Memory proactivity (storing important user information)
4. Conversation naturalness (not robotic, doesn't announce memory ops)
5. Following instructions correctly (done pattern, proper format)

**Bad Patterns to Check:** {bad_patterns}

Return JSON only:
{{"score": <0.0-1.0>, "feedback": "<detailed explanation of what was good/bad>"}}

Be strict but fair. A score of 1.0 means perfect. 0.8+ is good. 0.5-0.8 is acceptable. Below 0.5 has significant issues."#;

/// Format the judge prompt with example and response data
pub fn format_judge_prompt(example: &GepaExample, response: &ParsedResponse) -> String {
    let messages = if response.messages.is_empty() {
        "[]".to_string()
    } else {
        format!("{:?}", response.messages)
    };

    let tool_calls = if response.tool_calls.is_empty() {
        "[]".to_string()
    } else {
        response
            .tool_calls
            .iter()
            .map(|t| format!("{}({:?})", t.name, t.args))
            .collect::<Vec<_>>()
            .join(", ")
    };

    JUDGE_PROMPT
        .replace("{input}", &example.input)
        .replace("{expected_behavior}", &example.expected_behavior)
        .replace("{expected_response_type}", &example.expected_response_type)
        .replace("{expected_tools}", &format!("{:?}", example.expected_tools))
        .replace("{should_store_memory}", &example.should_store_memory.to_string())
        .replace("{reasoning}", &response.reasoning)
        .replace("{messages}", &messages)
        .replace("{tool_calls}", &tool_calls)
        .replace("{bad_patterns}", &format!("{:?}", example.bad_patterns))
}

/// Parse LLM judge response
pub fn parse_judge_response(response: &str) -> Result<EvaluationResult> {
    // Try to extract JSON from the response
    let json_str = if let Some(start) = response.find('{') {
        if let Some(end) = response.rfind('}') {
            &response[start..=end]
        } else {
            response
        }
    } else {
        response
    };

    #[derive(Deserialize)]
    struct JudgeOutput {
        score: f32,
        feedback: String,
    }

    let output: JudgeOutput = serde_json::from_str(json_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse judge response: {}", e))?;

    Ok(EvaluationResult {
        score: output.score.clamp(0.0, 1.0),
        feedback: output.feedback,
        component_scores: ComponentScores::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_example(
        response_type: &str,
        expected_tools: Vec<&str>,
        should_store: bool,
    ) -> GepaExample {
        GepaExample {
            id: "test".to_string(),
            category: "test".to_string(),
            input: "Test input".to_string(),
            previous_context_summary: "".to_string(),
            conversation_context: "".to_string(),
            expected_behavior: "Test".to_string(),
            expected_response_type: response_type.to_string(),
            expected_tools: expected_tools.into_iter().map(String::from).collect(),
            should_store_memory: should_store,
            bad_patterns: vec![],
        }
    }

    fn make_response(messages: Vec<&str>, tools: Vec<&str>) -> ParsedResponse {
        ParsedResponse {
            reasoning: "Test reasoning".to_string(),
            messages: messages.into_iter().map(String::from).collect(),
            tool_calls: tools
                .into_iter()
                .map(|name| ParsedToolCall {
                    name: name.to_string(),
                    args: std::collections::HashMap::new(),
                })
                .collect(),
            parse_error: None,
        }
    }

    #[test]
    fn test_casual_style() {
        let example = make_example("casual", vec![], false);
        let response = make_response(vec!["Hey!", "What's up?"], vec![]);
        let result = evaluate_rule_based(&example, &response);
        assert!(result.score > 0.7);
    }

    #[test]
    fn test_silent_done() {
        let example = make_example("silent_done", vec!["done"], false);
        let response = make_response(vec![], vec!["done"]);
        let result = evaluate_rule_based(&example, &response);
        assert!(result.score > 0.8);
    }

    #[test]
    fn test_tool_usage() {
        let example = make_example("tool_use", vec!["web_search"], false);
        let response = make_response(vec!["Let me search"], vec!["web_search"]);
        let result = evaluate_rule_based(&example, &response);
        assert!(result.component_scores.tools > 0.2);
    }

    #[test]
    fn test_memory_required() {
        let example = make_example("acknowledge_and_store", vec!["memory_append"], true);

        // Without memory
        let response1 = make_response(vec!["Great!"], vec![]);
        let result1 = evaluate_rule_based(&example, &response1);
        assert!(result1.component_scores.memory < 0.1);

        // With memory
        let response2 = make_response(vec!["Great!"], vec!["memory_append"]);
        let result2 = evaluate_rule_based(&example, &response2);
        assert!(result2.component_scores.memory > 0.15);
    }
}
