//! Shell command execution tool
//!
//! Allows Sage to execute arbitrary shell commands within its container.
//! Commands are run asynchronously with enforced timeouts. On timeout the
//! entire process group is killed so that child/background processes cannot
//! outlive the tool invocation and block the agent loop.
//!
//! When a command is killed due to timeout, any partial stdout/stderr captured
//! before the kill is included in the result so the agent can see what happened.

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::io::AsyncReadExt;
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

/// Maximum timeout in seconds (safety rail for clearly nonsensical values)
const MAX_TIMEOUT: u64 = 86_400; // 24 hours

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

    /// Read all available bytes from an optional pipe handle.
    /// Returns the content as a String (lossy UTF-8).
    async fn drain_pipe(pipe: &mut Option<tokio::process::ChildStdout>) -> String {
        // This generic approach won't work for ChildStderr directly, so we
        // have a separate overload below. Rust doesn't support trait-object
        // generics ergonomically here, so we just duplicate for the two types.
        if let Some(ref mut handle) = pipe {
            let mut buf = Vec::new();
            let _ = handle.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).into_owned()
        } else {
            String::new()
        }
    }

    /// Read all available bytes from an optional stderr pipe handle.
    async fn drain_stderr(pipe: &mut Option<tokio::process::ChildStderr>) -> String {
        if let Some(ref mut handle) = pipe {
            let mut buf = Vec::new();
            let _ = handle.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).into_owned()
        } else {
            String::new()
        }
    }

    /// Build the standard output string from stdout, stderr, and exit code.
    fn format_output(&self, stdout: &str, stderr: &str, exit_code: i32) -> String {
        let mut result_parts = Vec::new();

        if !stdout.is_empty() {
            result_parts.push(format!("STDOUT:\n{}", stdout.trim()));
        }

        if !stderr.is_empty() {
            result_parts.push(format!("STDERR:\n{}", stderr.trim()));
        }

        result_parts.push(format!("EXIT CODE: {}", exit_code));

        self.truncate_output(result_parts.join("\n\n"))
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
        "Execute a shell command in the workspace. Has access to CLI tools: git, curl, jq, grep, sed, awk, python3, node, etc. Use for file operations, running scripts, or system commands. Set the timeout parameter appropriately for each command (default 60s). If the command exceeds the timeout it will be killed and any partial output returned."
    }

    fn args_schema(&self) -> &str {
        r#"{"command": "shell command to execute (supports pipes, redirects)", "timeout": "optional timeout in seconds (default 60, set appropriately for long-running commands)"}"#
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
        let mut child = match Command::new("bash")
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

        // Take ownership of the pipe handles so we can read partial output on
        // timeout. child.wait() only waits for exit -- it does not consume the
        // pipes, unlike child.wait_with_output().
        let mut child_stdout = child.stdout.take();
        let mut child_stderr = child.stderr.take();
        // Note: child_stdout is Option<ChildStdout>, child_stderr is Option<ChildStderr>.
        // We use separate drain helpers because they are different types.
        let child_pid = child.id();

        match tokio::time::timeout(timeout_duration, child.wait()).await {
            Ok(Ok(status)) => {
                // Command finished within the timeout -- drain remaining output.
                let stdout = Self::drain_pipe(&mut child_stdout).await;
                let stderr = Self::drain_stderr(&mut child_stderr).await;
                let exit_code = status.code().unwrap_or(-1);

                let output_str = self.format_output(&stdout, &stderr, exit_code);

                debug!("Shell command completed with exit code {}", exit_code);

                Ok(ToolResult {
                    success: status.success(),
                    output: output_str,
                    error: if status.success() {
                        None
                    } else {
                        Some(format!("Command exited with code {}", exit_code))
                    },
                })
            }
            Ok(Err(e)) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to wait on command: {}", e)),
            }),
            Err(_) => {
                // Timeout -- kill the entire process group first, then drain
                // whatever partial output was written before the kill.
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

                // Reap the zombie so we don't leak it.
                let _ = child.wait().await;

                // Drain whatever was buffered in the pipes before the kill.
                let stdout = Self::drain_pipe(&mut child_stdout).await;
                let stderr = Self::drain_stderr(&mut child_stderr).await;

                let mut result_parts = Vec::new();

                if !stdout.is_empty() {
                    result_parts.push(format!("STDOUT (partial):\n{}", stdout.trim()));
                }
                if !stderr.is_empty() {
                    result_parts.push(format!("STDERR (partial):\n{}", stderr.trim()));
                }

                result_parts.push(format!(
                    "[Command timed out after {}s and was killed]",
                    timeout_secs
                ));

                let output_str = self.truncate_output(result_parts.join("\n\n"));

                Ok(ToolResult {
                    success: false,
                    output: output_str,
                    error: Some(format!("Command timed out after {}s", timeout_secs)),
                })
            }
        }
    }
}
