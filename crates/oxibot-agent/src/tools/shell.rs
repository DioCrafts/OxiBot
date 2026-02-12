//! Shell tool — execute commands in a subprocess.
//!
//! Port of nanobot's `agent/tools/shell.py` `ExecTool`.
//! Includes deny-pattern safety guard and optional workspace restriction.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use tokio::process::Command;
use tracing::{info, warn};

use super::base::{optional_string, require_string, Tool};

/// Maximum output length before truncation (characters).
const MAX_OUTPUT_LEN: usize = 10_000;

/// Default command timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Dangerous command patterns that are always blocked.
const DENY_PATTERNS: &[&str] = &[
    r"\brm\s+-[rf]{1,2}\b",
    r"\bdel\s+/[fq]\b",
    r"\brmdir\s+/s\b",
    r"\b(format|mkfs|diskpart)\b",
    r"\bdd\s+if=",
    r">\s*/dev/sd",
    r"\b(shutdown|reboot|poweroff)\b",
    r":\(\)\s*\{.*\};\s*:",   // fork bomb
];

// ─────────────────────────────────────────────
// ExecTool
// ─────────────────────────────────────────────

/// Execute shell commands in a subprocess.
pub struct ExecTool {
    /// Working directory for commands.
    working_dir: PathBuf,
    /// Command timeout.
    timeout: Duration,
    /// If true, block commands that reference paths outside `working_dir`.
    restrict_to_workspace: bool,
    /// Compiled deny regexes (built once at construction).
    deny_regexes: Vec<Regex>,
}

impl ExecTool {
    /// Create a new `ExecTool`.
    pub fn new(
        working_dir: PathBuf,
        timeout_secs: Option<u64>,
        restrict_to_workspace: bool,
    ) -> Self {
        let deny_regexes: Vec<Regex> = DENY_PATTERNS
            .iter()
            .filter_map(|p| Regex::new(p).ok())
            .collect();

        Self {
            working_dir,
            timeout: Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS)),
            restrict_to_workspace,
            deny_regexes,
        }
    }

    /// Check if a command is safe to execute. Returns an error message if blocked.
    fn guard_command(&self, command: &str, cwd: &str) -> Option<String> {
        let lower = command.to_lowercase();

        // Check deny patterns
        for re in &self.deny_regexes {
            if re.is_match(&lower) {
                warn!(command = command, "command blocked by safety guard");
                return Some(
                    "Error: Command blocked by safety guard (dangerous pattern detected)".into(),
                );
            }
        }

        // Workspace restriction
        if self.restrict_to_workspace {
            // Block path traversal
            if command.contains("../") || command.contains("..\\") {
                return Some(
                    "Error: Command blocked — path traversal (../) not allowed in restricted mode"
                        .into(),
                );
            }

            // Check for absolute paths outside workspace
            let cwd_path = PathBuf::from(cwd);
            let abs_path_re = Regex::new(r#"(?:/[^\s"']+|[A-Za-z]:\\[^\s"']+)"#).ok();
            if let Some(re) = abs_path_re {
                for cap in re.find_iter(command) {
                    let p = PathBuf::from(cap.as_str());
                    let resolved = if p.exists() {
                        p.canonicalize().unwrap_or(p)
                    } else {
                        p
                    };
                    if !resolved.starts_with(&cwd_path) {
                        return Some(format!(
                            "Error: Command references path '{}' outside workspace",
                            cap.as_str()
                        ));
                    }
                }
            }
        }

        None
    }
}

#[async_trait]
impl Tool for ExecTool {
    fn name(&self) -> &str {
        "exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. \
         Use this for running builds, tests, git, or any CLI tool."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Optional working directory (defaults to workspace root)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, params: HashMap<String, Value>) -> anyhow::Result<String> {
        let command = require_string(&params, "command")?;
        let cwd = optional_string(&params, "working_dir")
            .unwrap_or_else(|| self.working_dir.to_string_lossy().to_string());

        // Safety check
        if let Some(err) = self.guard_command(&command, &cwd) {
            return Ok(err); // return as tool output, not Rust error
        }

        info!(command = %command, cwd = %cwd, "executing shell command");

        // Spawn the process
        let child = Command::new(if cfg!(target_os = "windows") { "cmd" } else { "sh" })
            .args(if cfg!(target_os = "windows") {
                vec!["/C", &command]
            } else {
                vec!["-c", &command]
            })
            .current_dir(&cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to spawn command: {e}"))?;

        // Wait with timeout
        let result = tokio::time::timeout(self.timeout, child.wait_with_output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let code = output.status.code().unwrap_or(-1);

                let mut parts = Vec::new();
                if !stdout.is_empty() {
                    parts.push(stdout);
                }
                if !stderr.is_empty() {
                    parts.push(format!("STDERR:\n{stderr}"));
                }
                if code != 0 {
                    parts.push(format!("Exit code: {code}"));
                }

                let mut combined = if parts.is_empty() {
                    "(no output)".to_string()
                } else {
                    parts.join("\n")
                };

                // Truncate if too long
                if combined.len() > MAX_OUTPUT_LEN {
                    let remaining = combined.len() - MAX_OUTPUT_LEN;
                    combined.truncate(MAX_OUTPUT_LEN);
                    combined.push_str(&format!("\n... (truncated, {remaining} more chars)"));
                }

                Ok(combined)
            }
            Ok(Err(e)) => {
                anyhow::bail!("Command failed: {e}");
            }
            Err(_) => {
                // Timeout
                Ok(format!(
                    "Error: Command timed out after {} seconds",
                    self.timeout.as_secs()
                ))
            }
        }
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_params(pairs: &[(&str, &str)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect()
    }

    #[tokio::test]
    async fn test_exec_echo() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ExecTool::new(dir.path().to_path_buf(), Some(10), false);
        let result = tool
            .execute(make_params(&[("command", "echo hello")]))
            .await
            .unwrap();
        assert!(result.contains("hello"));
    }

    #[tokio::test]
    async fn test_exec_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ExecTool::new(dir.path().to_path_buf(), Some(10), false);
        let result = tool
            .execute(make_params(&[("command", "exit 42")]))
            .await
            .unwrap();
        assert!(result.contains("Exit code: 42"));
    }

    #[test]
    fn test_guard_blocks_rm_rf() {
        let tool = ExecTool::new(PathBuf::from("/tmp"), None, false);
        let guard = tool.guard_command("rm -rf /", "/tmp");
        assert!(guard.is_some());
        assert!(guard.unwrap().contains("dangerous pattern"));
    }

    #[test]
    fn test_guard_blocks_fork_bomb() {
        let tool = ExecTool::new(PathBuf::from("/tmp"), None, false);
        let guard = tool.guard_command(":() { :|:& };:", "/tmp");
        assert!(guard.is_some());
    }

    #[test]
    fn test_guard_blocks_shutdown() {
        let tool = ExecTool::new(PathBuf::from("/tmp"), None, false);
        let guard = tool.guard_command("sudo shutdown -h now", "/tmp");
        assert!(guard.is_some());
    }

    #[test]
    fn test_guard_allows_safe_commands() {
        let tool = ExecTool::new(PathBuf::from("/tmp"), None, false);
        assert!(tool.guard_command("echo hello", "/tmp").is_none());
        assert!(tool.guard_command("ls -la", "/tmp").is_none());
        assert!(tool.guard_command("cat file.txt", "/tmp").is_none());
        assert!(tool.guard_command("cargo test", "/tmp").is_none());
    }

    #[test]
    fn test_guard_blocks_traversal_in_restricted_mode() {
        let tool = ExecTool::new(PathBuf::from("/tmp/workspace"), None, true);
        let guard = tool.guard_command("cat ../../../etc/passwd", "/tmp/workspace");
        assert!(guard.is_some());
        assert!(guard.unwrap().contains("path traversal"));
    }

    #[tokio::test]
    async fn test_exec_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ExecTool::new(dir.path().to_path_buf(), Some(1), false);
        let result = tool
            .execute(make_params(&[("command", "sleep 30")]))
            .await
            .unwrap();
        assert!(result.contains("timed out"));
    }

    #[test]
    fn test_tool_definition() {
        let tool = ExecTool::new(PathBuf::from("/tmp"), None, false);
        let def = tool.to_definition();
        assert_eq!(def.function.name, "exec");
        assert_eq!(def.tool_type, "function");
    }
}
