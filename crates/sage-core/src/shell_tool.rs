//! Shell command execution tool
//!
//! Allows Sage to execute arbitrary shell commands within its container.
//! Commands are run asynchronously with enforced timeouts. On timeout the
//! entire process group is killed so that child/background processes cannot
//! outlive the tool invocation and block the agent loop.

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::sage_agent::{Tool, ToolResult};

/// Dangerous command patterns that should be blocked
const BLOCKED_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    "rm -rf ~",
    "mkfs",
    "dd if=",
    ":(){:|:&};:", // Fork bomb
    "> /dev/sd",
    "chmod -R 777 /",
    "shutdown",
    "reboot",
    "init 0",
    "init 6",
];

/// Maximum output size in bytes
const MAX_OUTPUT_SIZE: usize = 100_000; // 100KB

/// Default timeout in seconds
const DEFAULT_TIMEOUT: u64 = 60;

/// Maximum timeout in seconds  
const MAX_TIMEOUT: u64 = 300;

/// Shell command execution tool
pub struct ShellTool {
    workspace: String,
}

impl ShellTool {
    pub fn new(workspace: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }

    /// Check if a command contains blocked patterns
    fn is_blocked(&self, command: &str) -> Option<&'static str> {
        let lower = command.to_lowercase();
        BLOCKED_PATTERNS
            .iter()
            .find(|&pattern| lower.contains(pattern))
            .copied()
    }

    /// Truncate output if too long (handles UTF-8 boundaries safely)
    fn truncate_output(&self, output: String) -> String {
        if output.len() > MAX_OUTPUT_SIZE {
            // Find a valid UTF-8 char boundary near MAX_OUTPUT_SIZE
            let mut end = MAX_OUTPUT_SIZE;
            while !output.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            format!(
                "{}\n\n[OUTPUT TRUNCATED - exceeded {} bytes, showing first {}]",
                &output[..end],
                output.len(),
                end
            )
        } else {
            output
        }
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Execute a shell command in the workspace. Has access to CLI tools: git, curl, jq, grep, sed, awk, python3, node, etc. Use for file operations, running scripts, or system commands. Commands MUST complete within the timeout -- do NOT launch persistent/background daemons (they will be killed when the timeout expires)."
    }

    fn args_schema(&self) -> &str {
        r#"{"command": "shell command to execute (supports pipes, redirects)", "timeout": "optional timeout in seconds (default 60, max 300)"}"#
    }

    async fn execute(&self, args: &HashMap<String, String>) -> Result<ToolResult> {
        let command = args
            .get("command")
            .ok_or_else(|| anyhow::anyhow!("'command' argument is required"))?;

        let timeout_secs: u64 = args
            .get("timeout")
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_TIMEOUT)
            .min(MAX_TIMEOUT);

        info!(
            "Executing shell command: {} (timeout: {}s)",
            command, timeout_secs
        );

        // Check for blocked patterns
        if let Some(pattern) = self.is_blocked(command) {
            warn!("Blocked dangerous command pattern: {}", pattern);
            return Ok(ToolResult {
                success: false,
                output: format!("Command blocked: contains dangerous pattern '{}'", pattern),
                error: Some("Security violation".to_string()),
            });
        }

        // Ensure workspace exists
        std::fs::create_dir_all(&self.workspace).ok();

        // Spawn command in a new process group so we can kill the entire tree
        // (including any child/background processes) on timeout.
        let child = match Command::new("bash")
            .args(["-c", command])
            .current_dir(&self.workspace)
            .env("HOME", &self.workspace)
            .env("PWD", &self.workspace)
            .process_group(0)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to execute command: {}", e)),
                });
            }
        };

        let timeout_duration = std::time::Duration::from_secs(timeout_secs);

        // Grab the PID now -- wait_with_output() consumes the child.
        let child_pid = child.id();

        match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);

                let mut result_parts = Vec::new();

                if !stdout.is_empty() {
                    result_parts.push(format!("STDOUT:\n{}", stdout.trim()));
                }

                if !stderr.is_empty() {
                    result_parts.push(format!("STDERR:\n{}", stderr.trim()));
                }

                result_parts.push(format!("EXIT CODE: {}", exit_code));

                let output_str = self.truncate_output(result_parts.join("\n\n"));

                debug!("Shell command completed with exit code {}", exit_code);

                Ok(ToolResult {
                    success: output.status.success(),
                    output: output_str,
                    error: if output.status.success() {
                        None
                    } else {
                        Some(format!("Command exited with code {}", exit_code))
                    },
                })
            }
            Ok(Err(e)) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to read command output: {}", e)),
            }),
            Err(_) => {
                // Timeout -- kill the entire process group
                warn!(
                    "Shell command timed out after {}s, killing process group: {}",
                    timeout_secs, command
                );

                if let Some(pid) = child_pid {
                    let pgid = pid as i32;
                    // SIGKILL the entire process group (negative pid)
                    unsafe {
                        libc::kill(-pgid, libc::SIGKILL);
                    }
                }

                Ok(ToolResult {
                    success: false,
                    output: format!(
                        "Command timed out after {} seconds and was killed (including all child processes). \
                         Do NOT launch persistent daemons or background services via this tool -- \
                         they will always be killed at timeout.",
                        timeout_secs
                    ),
                    error: Some(format!("Command timed out after {}s", timeout_secs)),
                })
            }
        }
    }
}
