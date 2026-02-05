# GEPA Prompt Optimization for Sage

This document describes how to use GEPA (Genetic-Pareto) to automatically optimize Sage's agent instructions through reflective prompt evolution.

## Overview

GEPA is a state-of-the-art prompt optimizer that uses:
1. **Rich Textual Feedback** - Not just scores, but detailed explanations of what went wrong
2. **Pareto-based Selection** - Maintains diverse candidates that excel on different examples
3. **LLM-driven Reflection** - Uses an LLM to analyze execution traces and propose improvements
4. **Evolutionary Search** - Iteratively improves prompts through mutation and selection

Reference: "GEPA: Reflective Prompt Evolution Can Outperform Reinforcement Learning" (Agrawal et al., 2025, arxiv:2507.19457)

## Why GEPA for Sage?

Sage uses DSRs (dspy-rs) signatures with typed inputs/outputs. The current `AGENT_INSTRUCTION` constant (~160 lines) was manually engineered. GEPA can:

1. **Discover domain-specific patterns** we might not think of
2. **Optimize for actual failure modes** observed in evaluation
3. **Reduce instruction length** while maintaining performance
4. **Adapt to different scenarios** (casual chat, tool use, memory operations)

## Architecture

### Current Sage Structure

```
┌─────────────────────────────────────────────────────────────────┐
│ AgentResponse Signature                                         │
├─────────────────────────────────────────────────────────────────┤
│ Inputs:                                                         │
│   - input: User message or tool result                          │
│   - previous_context_summary: Old message summary               │
│   - conversation_context: Recent history + memory blocks        │
│   - available_tools: Tool descriptions                          │
│                                                                 │
│ Outputs:                                                        │
│   - reasoning: Thought process                                  │
│   - messages: Array of messages to send                         │
│   - tool_calls: Array of tool calls                             │
└─────────────────────────────────────────────────────────────────┘
                              ▲
                              │
                    AGENT_INSTRUCTION
                    (this is what GEPA optimizes)
```

### GEPA Optimization Loop

```
┌─────────────────────────────────────────────────────────────────┐
│ GEPA Evolutionary Process                                       │
├─────────────────────────────────────────────────────────────────┤
│  1. Initialize Pareto frontier with seed instruction            │
│  2. For each iteration:                                         │
│     a) Sample candidate from frontier (proportional to wins)    │
│     b) Run minibatch through module → collect traces            │
│     c) LLM REFLECTION: Analyze traces, identify weaknesses      │
│     d) LLM MUTATION: Propose improved instruction               │
│     e) Evaluate new instruction on trainset/valset              │
│     f) Add to frontier if non-dominated                         │
│  3. Return best candidate by average score                      │
└─────────────────────────────────────────────────────────────────┘
```

## Implementation Plan

### Phase 1: Minimal Seed Instruction

Start with a simple instruction to give GEPA room to explore:

```rust
pub const MINIMAL_SEED_INSTRUCTION: &str = r#"You are Sage, a helpful AI assistant communicating via Signal.

You have access to memory (core blocks, archival storage) and tools (web search, etc.).
Respond naturally. Use tools when helpful. Store important information to memory proactively.

For casual chat, use multiple short messages. For detailed explanations, longer messages are fine.
"#;
```

The detailed instruction in `sage_agent.rs` is preserved in git and can be restored or used as a baseline comparison.

### Phase 2: Evaluation Dataset

We need 20-30 diverse examples covering Sage's capabilities:

#### Example Categories

1. **Casual Greetings** - Should respond with multiple short messages, no tools
2. **Information Questions** - May need web search, then summarize
3. **Memory Recall** - Should search archival/conversation history
4. **New Information Storage** - Should proactively store to memory
5. **Tool Result Processing** - Should summarize search results
6. **Memory Operation Completion** - Should return `done` silently
7. **Multi-turn Context** - Should maintain conversation coherence
8. **Complex Requests** - May need multiple tools in sequence

#### Example Format

Each example should specify:
- `input`: The user message or tool result
- `conversation_context`: Prior conversation history
- `expected_behavior`: What the agent should do
- `expected_response_type`: casual/detailed/tool_use/silent_done
- `expected_tools`: List of tool names that should be called
- `should_store_memory`: Whether memory storage is expected

### Phase 3: FeedbackEvaluator Implementation

Two approaches:

#### Option A: Rule-Based Feedback (Faster, Cheaper)

```rust
impl FeedbackEvaluator for SageModule {
    async fn feedback_metric(&self, example: &Example, prediction: &Prediction) -> FeedbackMetric {
        let mut score = 0.0;
        let mut feedback = String::new();
        
        // 1. Format correctness (0.2)
        // 2. Response style match (0.2)
        // 3. Tool usage correctness (0.3)
        // 4. Memory proactivity (0.2)
        // 5. Content quality (0.1)
        
        FeedbackMetric::new(score, feedback)
    }
}
```

#### Option B: LLM-as-Judge (More Nuanced, Expensive)

Use Claude Sonnet 4.5 to evaluate responses with a judge prompt:

```rust
async fn llm_judge_feedback(
    input: &str,
    expected_behavior: &str,
    actual_response: &AgentResponseOutput,
) -> FeedbackMetric {
    let judge_prompt = format!(r#"
Evaluate this AI assistant response. Score 0.0-1.0 with detailed feedback.

**Input:** {input}
**Expected Behavior:** {expected_behavior}
**Actual Response:**
- Reasoning: {reasoning}
- Messages: {messages:?}
- Tool Calls: {tools:?}

**Evaluation Criteria:**
1. Response appropriateness (casual vs detailed)
2. Correct tool usage (right tools, right time)
3. Memory proactivity (storing important info)
4. Conversation naturalness
5. Following instructions (done pattern, no duplicate fields)

Return JSON: {{"score": 0.0-1.0, "feedback": "detailed explanation"}}
"#, ...);

    // Call judge LLM and parse response
}
```

### Phase 4: GEPA Configuration

```rust
let gepa = GEPA::builder()
    .num_iterations(15)           // Evolutionary generations
    .minibatch_size(10)           // Examples per iteration
    .num_trials(5)                // Evaluation trials per candidate
    .temperature(0.9)             // Mutation creativity
    .track_stats(true)            // Track evolution history
    .maybe_max_rollouts(Some(500))// Budget control
    .maybe_prompt_model(Some(     // Use Claude for reflection/mutation
        LM::builder()
            .model("claude-sonnet-4-5-20250514")
            .build().await?
    ))
    .build();
```

### Phase 5: Running Optimization

```bash
# Run GEPA optimization
just gepa-optimize --iterations 15 --examples examples/gepa_trainset.json

# Compare baseline vs optimized
just gepa-compare baseline.txt optimized.txt --examples examples/gepa_valset.json
```

## Evaluation Dataset Examples

### 1. Casual Greeting

```json
{
  "input": "Hey! How's it going?",
  "previous_context_summary": "",
  "conversation_context": "Current time: 02/04/2026 10:00:00 (Tuesday)\n<memory_blocks>...</memory_blocks>",
  "expected_behavior": "Respond with friendly, casual greeting. Multiple short messages. No tools needed.",
  "expected_response_type": "casual",
  "expected_tools": [],
  "should_store_memory": false
}
```

**Good Response:**
```
reasoning: "User is greeting me casually. Should respond warmly with short messages."
messages: ["Hey!", "Going well, thanks for asking!", "What's up?"]
tool_calls: []
```

**Bad Response:**
```
messages: ["Hello! I am doing well. How can I assist you today with any questions or tasks you might have?"]
tool_calls: []
```
Feedback: "Style Mismatch: Expected casual multi-message response, got single formal message."

### 2. Web Search Request

```json
{
  "input": "What's the latest news on Bitcoin?",
  "previous_context_summary": "",
  "conversation_context": "...",
  "expected_behavior": "Use web_search tool to find current Bitcoin news.",
  "expected_response_type": "tool_use",
  "expected_tools": ["web_search"],
  "should_store_memory": false
}
```

**Good Response:**
```
reasoning: "User wants current Bitcoin news. I should search the web for this."
messages: ["Let me check the latest Bitcoin news for you."]
tool_calls: [{"name": "web_search", "args": {"query": "Bitcoin news today 2026"}}]
```

### 3. Tool Result Processing (Web Search)

```json
{
  "input": "[Tool Result: web_search]\nQuery: Bitcoin news today\nResults:\n1. Bitcoin Hits $150,000 Milestone - CoinDesk\n   Bitcoin reached a new all-time high...\n2. ETF Inflows Continue - Bloomberg\n   Institutional investment remains strong...",
  "conversation_context": "User: What's the latest news on Bitcoin?\nAssistant: Let me search for that...",
  "expected_behavior": "Summarize the search results naturally. No more tools needed.",
  "expected_response_type": "summarize_results",
  "expected_tools": [],
  "should_store_memory": false
}
```

**Good Response:**
```
reasoning: "Got search results about Bitcoin. Should summarize the key news points."
messages: ["Here's what's happening with Bitcoin:", "It just hit $150,000 - a new all-time high!", "Institutional investment through ETFs is still going strong according to Bloomberg."]
tool_calls: []
```

**Bad Response:**
```
messages: []
tool_calls: [{"name": "done", "args": {}}]
```
Feedback: "Missing Response: User asked about Bitcoin news and received search results. Should summarize findings, not return done."

### 4. Memory Storage (New Information)

```json
{
  "input": "I just got a new job at Google as a software engineer!",
  "previous_context_summary": "",
  "conversation_context": "...",
  "expected_behavior": "Congratulate user AND proactively store this important life event to memory.",
  "expected_response_type": "acknowledge_and_store",
  "expected_tools": ["memory_append", "archival_insert"],
  "should_store_memory": true
}
```

**Good Response:**
```
reasoning: "User shared exciting career news. Should congratulate them and store this to memory."
messages: ["That's amazing news, congratulations!", "Google is a great company. What team will you be joining?"]
tool_calls: [
  {"name": "memory_append", "args": {"block": "human", "content": "Works at Google as a software engineer (started ~Feb 2026)"}},
  {"name": "archival_insert", "args": {"content": "User got a new job at Google as a software engineer in February 2026. This is a significant career milestone."}}
]
```

**Bad Response:**
```
messages: ["Congratulations! That's great news!"]
tool_calls: []
```
Feedback: "Memory Failure: User shared important life event (new job at Google). Should have stored to memory proactively."

### 5. Memory Operation Completion

```json
{
  "input": "[Tool Result: memory_append]\nSuccessfully appended to <human> block.",
  "conversation_context": "User: I just got a new job at Google!\nAssistant: Congratulations! [tool: memory_append]",
  "expected_behavior": "Memory was stored silently. Return done - user doesn't need confirmation.",
  "expected_response_type": "silent_done",
  "expected_tools": ["done"],
  "should_store_memory": false
}
```

**Good Response:**
```
reasoning: "Memory operation completed. User already received congratulations. Nothing more to say."
messages: []
tool_calls: [{"name": "done", "args": {}}]
```

**Bad Response:**
```
messages: ["I've saved that information to my memory!"]
tool_calls: []
```
Feedback: "Unnecessary Announcement: Memory operations should be silent. Don't tell user you saved to memory."

### 6. Archival Search Request

```json
{
  "input": "What do you remember about my trip to Japan?",
  "previous_context_summary": "",
  "conversation_context": "...",
  "expected_behavior": "Search archival memory for Japan trip information.",
  "expected_response_type": "tool_use",
  "expected_tools": ["archival_search"],
  "should_store_memory": false
}
```

### 7. Complex Multi-Tool Request

```json
{
  "input": "Can you find the current weather in Tokyo and remind me what I said about wanting to visit there?",
  "previous_context_summary": "",
  "conversation_context": "...",
  "expected_behavior": "Needs both web_search for weather AND archival_search for past mentions.",
  "expected_response_type": "tool_use",
  "expected_tools": ["web_search", "archival_search"],
  "should_store_memory": false
}
```

### 8. Follow-up Question (Context Awareness)

```json
{
  "input": "What about the price?",
  "conversation_context": "User: Tell me about the iPhone 16\nAssistant: The iPhone 16 features... [detailed response about features]",
  "expected_behavior": "Understand 'price' refers to iPhone 16 from context. May need web search.",
  "expected_response_type": "context_aware",
  "expected_tools": ["web_search"],
  "should_store_memory": false
}
```

## Feedback Quality Guidelines

### Good Feedback (Actionable)

```
Incorrect tool usage
  Expected: ["web_search"]
  Got: []
  Issue: User asked about current news which requires web search, but no search was performed.
  Suggestion: When users ask about "latest", "current", or "news", use web_search.
```

### Bad Feedback (Vague)

```
Wrong answer. Score: 0.0
```

### Feedback Template

```
{category} {status}
  Expected: {expected}
  Got: {actual}
  Issue: {specific explanation of what went wrong}
  Suggestion: {how to improve}
```

## Running GEPA

### Prerequisites

1. DSRs with GEPA support (check Cargo.toml)
2. Anthropic API key for Claude Sonnet 4.5 (judge + mutation LLM)
3. Evaluation dataset in JSON format

### Commands

```bash
# Create evaluation dataset
just gepa-create-dataset

# Run optimization (development - small budget)
just gepa-optimize-dev

# Run optimization (production - full budget)
just gepa-optimize

# View evolution history
just gepa-history

# Compare instructions
just gepa-compare --baseline current --optimized optimized_v1
```

### Expected Output

```
GEPA: Starting reflective prompt optimization
  Iterations: 15
  Minibatch size: 10
  Initialized frontier with 1 candidates

Generation 1/15
  Sampled parent (ID 0): avg score 0.650
  Collected 10 traces
  Reflection: "The instruction doesn't clearly specify when to use multiple messages..."
  Generated new instruction through reflection
  Child avg score: 0.720
  Added to Pareto frontier
  Frontier size: 2

Generation 2/15
  ...

GEPA optimization complete
  Best average score: 0.890
  Total rollouts: 150
  Total LM calls: 30

Evolution History:
  Generation 0: 0.650
  Generation 1: 0.720
  Generation 5: 0.810
  Generation 10: 0.870
  Generation 15: 0.890

Best Instruction:
  "You are Sage, a helpful AI assistant on Signal.
   
   RESPONSE STYLE:
   - Casual greetings → 2-3 short messages
   - Questions needing research → acknowledge + web_search
   - After search results → summarize naturally, don't just return done
   ..."
```

## Comparing Instructions

### Baseline (Manual, ~160 lines)
- Comprehensive coverage of all scenarios
- Explicit rules for every case
- May be over-specified

### GEPA Optimized (Evolved)
- Discovered through evaluation feedback
- Focuses on actual failure modes
- Often shorter but more precise
- May find surprising patterns we didn't think of

### A/B Testing

After optimization, run both instructions on a held-out test set:

```bash
just gepa-ab-test --baseline manual --optimized gepa_v1 --testset test_examples.json
```

## Troubleshooting

### Low Scores Across All Candidates
- Check evaluation dataset quality
- Ensure feedback is specific and actionable
- May need more diverse examples

### No Improvement Over Generations
- Increase temperature for more exploration
- Check if feedback is too vague
- May need more iterations

### Optimization Too Slow
- Reduce minibatch_size
- Use smaller judge model
- Set max_rollouts budget

### Pareto Frontier Gets Too Large
- Candidates are too diverse (good problem!)
- Focus on average score for final selection

## Files

```
sage/
├── crates/
│   └── sage-core/
│       └── src/
│           ├── sage_agent.rs          # Current agent with AGENT_INSTRUCTION
│           ├── gepa/
│           │   ├── mod.rs             # GEPA module wrapper
│           │   ├── evaluator.rs       # FeedbackEvaluator implementation
│           │   ├── dataset.rs         # Evaluation dataset loading
│           │   └── judge.rs           # LLM-as-Judge implementation
│           └── bin/
│               └── gepa_optimize.rs   # CLI for running optimization
├── examples/
│   └── gepa/
│       ├── trainset.json              # Training examples
│       ├── valset.json                # Validation examples
│       └── testset.json               # Held-out test examples
├── docs/
│   └── GEPA_OPTIMIZATION.md           # This document
└── optimized_instructions/
    ├── baseline.txt                   # Original manual instruction
    └── gepa_v1.txt                    # GEPA-optimized instruction
```

## Next Steps

1. [ ] Implement GEPA module wrapper in `crates/sage-core/src/gepa/`
2. [ ] Create FeedbackEvaluator with LLM-as-Judge
3. [ ] Build evaluation dataset (20-30 examples)
4. [ ] Add CLI commands to justfile
5. [ ] Run initial optimization with small budget
6. [ ] Analyze results and iterate
7. [ ] A/B test optimized vs baseline
8. [ ] If better, consider replacing AGENT_INSTRUCTION

## References

- [GEPA Paper](https://arxiv.org/abs/2507.19457)
- [GEPA GitHub](https://github.com/gepa-ai/gepa)
- [DSRs GEPA Implementation](https://github.com/krypticmouse/DSRs/blob/main/crates/dspy-rs/src/optimizer/gepa.rs)
- [DSRs GEPA Example](https://github.com/krypticmouse/DSRs/blob/main/crates/dspy-rs/examples/09-gepa-sentiment.rs)
