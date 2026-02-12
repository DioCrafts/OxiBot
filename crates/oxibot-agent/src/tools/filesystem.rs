//! Filesystem tools — read, write, edit, list directory.
//!
//! Port of nanobot's `agent/tools/filesystem.py`.
//! Each tool optionally restricts paths to an `allowed_dir`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::base::{require_string, Tool};

// ─────────────────────────────────────────────
// Shared path helper
// ─────────────────────────────────────────────

/// Resolve a user-supplied path, optionally restricting it to `allowed_dir`.
///
/// Returns `Err` if the resolved path is outside the allowed directory.
fn resolve_path(path: &str, allowed_dir: Option<&Path>) -> anyhow::Result<PathBuf> {
    // Expand ~ to home directory
    let expanded = if path.starts_with("~/") || path == "~" {
        if let Some(home) = dirs_like_home() {
            home.join(&path[2..])
        } else {
            PathBuf::from(path)
        }
    } else {
        PathBuf::from(path)
    };

    // Canonicalize if the path exists, otherwise use the expanded form
    let resolved = if expanded.exists() {
        expanded.canonicalize().unwrap_or(expanded)
    } else {
        // For write operations the file may not exist yet;
        // canonicalize the parent if possible.
        if let Some(parent) = expanded.parent() {
            if parent.exists() {
                let canon_parent = parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf());
                if let Some(name) = expanded.file_name() {
                    canon_parent.join(name)
                } else {
                    expanded
                }
            } else {
                expanded
            }
        } else {
            expanded
        }
    };

    // Enforce allowed_dir restriction
    if let Some(allowed) = allowed_dir {
        let allowed_canon = if allowed.exists() {
            allowed.canonicalize().unwrap_or_else(|_| allowed.to_path_buf())
        } else {
            allowed.to_path_buf()
        };
        if !resolved.starts_with(&allowed_canon) {
            anyhow::bail!(
                "Access denied: path '{}' is outside allowed directory '{}'",
                resolved.display(),
                allowed_canon.display()
            );
        }
    }

    Ok(resolved)
}

/// Best-effort home directory (avoids pulling in the `dirs` crate).
fn dirs_like_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

// ─────────────────────────────────────────────
// ReadFileTool
// ─────────────────────────────────────────────

/// Reads and returns the entire content of a file.
pub struct ReadFileTool {
    allowed_dir: Option<PathBuf>,
}

impl ReadFileTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file at the given path. Returns the full text content."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file to read"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: HashMap<String, Value>) -> anyhow::Result<String> {
        let path_str = require_string(&params, "path")?;
        let path = resolve_path(&path_str, self.allowed_dir.as_deref())?;

        if !path.exists() {
            anyhow::bail!("File not found: {}", path.display());
        }
        if !path.is_file() {
            anyhow::bail!("Not a file: {}", path.display());
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", path.display()))?;
        Ok(content)
    }
}

// ─────────────────────────────────────────────
// WriteFileTool
// ─────────────────────────────────────────────

/// Creates or overwrites a file with the given content.
pub struct WriteFileTool {
    allowed_dir: Option<PathBuf>,
}

impl WriteFileTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file, creating it if it doesn't exist or overwriting if it does. \
         Parent directories are created automatically."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path for the file"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, params: HashMap<String, Value>) -> anyhow::Result<String> {
        let path_str = require_string(&params, "path")?;
        let content = require_string(&params, "content")?;
        let path = resolve_path(&path_str, self.allowed_dir.as_deref())?;

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow::anyhow!("Failed to create directory {}: {e}", parent.display()))?;
            }
        }

        let bytes = content.as_bytes().len();
        std::fs::write(&path, &content)
            .map_err(|e| anyhow::anyhow!("Failed to write {}: {e}", path.display()))?;
        Ok(format!("Successfully wrote {bytes} bytes to {}", path.display()))
    }
}

// ─────────────────────────────────────────────
// EditFileTool
// ─────────────────────────────────────────────

/// Replaces a text snippet within a file (single occurrence).
pub struct EditFileTool {
    allowed_dir: Option<PathBuf>,
}

impl EditFileTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing the first occurrence of `old_text` with `new_text`. \
         Include enough context in `old_text` to uniquely identify the replacement site."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "old_text": {
                    "type": "string",
                    "description": "Exact text to find (include surrounding context for uniqueness)"
                },
                "new_text": {
                    "type": "string",
                    "description": "Text to replace old_text with"
                }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    async fn execute(&self, params: HashMap<String, Value>) -> anyhow::Result<String> {
        let path_str = require_string(&params, "path")?;
        let old_text = require_string(&params, "old_text")?;
        let new_text = require_string(&params, "new_text")?;
        let path = resolve_path(&path_str, self.allowed_dir.as_deref())?;

        if !path.is_file() {
            anyhow::bail!("File not found: {}", path.display());
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", path.display()))?;

        let count = content.matches(&old_text).count();
        if count == 0 {
            anyhow::bail!("old_text not found in {}", path.display());
        }

        let mut warning = String::new();
        if count > 1 {
            warning = format!(
                "Warning: old_text appears {count} times; only the first occurrence was replaced. "
            );
        }

        // Replace exactly one occurrence
        let updated = content.replacen(&old_text, &new_text, 1);
        std::fs::write(&path, &updated)
            .map_err(|e| anyhow::anyhow!("Failed to write {}: {e}", path.display()))?;

        Ok(format!(
            "{warning}Successfully edited {}",
            path.display()
        ))
    }
}

// ─────────────────────────────────────────────
// ListDirTool
// ─────────────────────────────────────────────

/// Lists the contents of a directory.
pub struct ListDirTool {
    allowed_dir: Option<PathBuf>,
}

impl ListDirTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List the contents of a directory. Returns file and folder names with type indicators."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the directory to list"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: HashMap<String, Value>) -> anyhow::Result<String> {
        let path_str = require_string(&params, "path")?;
        let path = resolve_path(&path_str, self.allowed_dir.as_deref())?;

        if !path.is_dir() {
            anyhow::bail!("Not a directory: {}", path.display());
        }

        let mut entries: Vec<String> = Vec::new();
        let mut dir_entries: Vec<_> = std::fs::read_dir(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read directory {}: {e}", path.display()))?
            .filter_map(|e| e.ok())
            .collect();

        // Sort by name
        dir_entries.sort_by_key(|e| e.file_name());

        for entry in dir_entries {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry
                .file_type()
                .map(|ft| ft.is_dir())
                .unwrap_or(false);
            if is_dir {
                entries.push(format!("📁 {name}"));
            } else {
                entries.push(format!("📄 {name}"));
            }
        }

        if entries.is_empty() {
            Ok("(empty directory)".into())
        } else {
            Ok(entries.join("\n"))
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

    // ── ReadFileTool ──

    #[tokio::test]
    async fn test_read_file_success() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        std::fs::write(&file, "Hello, Oxibot!").unwrap();

        let tool = ReadFileTool::new(None);
        let result = tool
            .execute(make_params(&[("path", file.to_str().unwrap())]))
            .await
            .unwrap();
        assert_eq!(result, "Hello, Oxibot!");
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let tool = ReadFileTool::new(None);
        let result = tool
            .execute(make_params(&[("path", "/tmp/nonexistent_oxibot_test_file.txt")]))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_read_file_restricted() {
        let dir = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("safe");
        std::fs::create_dir(&allowed).unwrap();
        let outside = dir.path().join("secret.txt");
        std::fs::write(&outside, "nope").unwrap();

        let tool = ReadFileTool::new(Some(allowed));
        let result = tool
            .execute(make_params(&[("path", outside.to_str().unwrap())]))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Access denied"));
    }

    // ── WriteFileTool ──

    #[tokio::test]
    async fn test_write_file_create() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("output.txt");

        let tool = WriteFileTool::new(None);
        let result = tool
            .execute(make_params(&[
                ("path", file.to_str().unwrap()),
                ("content", "Written content"),
            ]))
            .await
            .unwrap();
        assert!(result.contains("Successfully wrote"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "Written content");
    }

    #[tokio::test]
    async fn test_write_file_creates_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub").join("deep").join("file.txt");

        let tool = WriteFileTool::new(None);
        let result = tool
            .execute(make_params(&[
                ("path", file.to_str().unwrap()),
                ("content", "deep content"),
            ]))
            .await
            .unwrap();
        assert!(result.contains("Successfully wrote"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "deep content");
    }

    // ── EditFileTool ──

    #[tokio::test]
    async fn test_edit_file_success() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("edit.txt");
        std::fs::write(&file, "Hello World").unwrap();

        let tool = EditFileTool::new(None);
        let result = tool
            .execute(make_params(&[
                ("path", file.to_str().unwrap()),
                ("old_text", "World"),
                ("new_text", "Oxibot"),
            ]))
            .await
            .unwrap();
        assert!(result.contains("Successfully edited"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "Hello Oxibot");
    }

    #[tokio::test]
    async fn test_edit_file_not_found_text() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("edit2.txt");
        std::fs::write(&file, "ABC").unwrap();

        let tool = EditFileTool::new(None);
        let result = tool
            .execute(make_params(&[
                ("path", file.to_str().unwrap()),
                ("old_text", "XYZ"),
                ("new_text", "123"),
            ]))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_file_multiple_occurrences_warning() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("multi.txt");
        std::fs::write(&file, "aaa bbb aaa").unwrap();

        let tool = EditFileTool::new(None);
        let result = tool
            .execute(make_params(&[
                ("path", file.to_str().unwrap()),
                ("old_text", "aaa"),
                ("new_text", "ccc"),
            ]))
            .await
            .unwrap();
        assert!(result.contains("Warning"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "ccc bbb aaa");
    }

    // ── ListDirTool ──

    #[tokio::test]
    async fn test_list_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file_a.txt"), "").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let tool = ListDirTool::new(None);
        let result = tool
            .execute(make_params(&[("path", dir.path().to_str().unwrap())]))
            .await
            .unwrap();
        assert!(result.contains("📄 file_a.txt"));
        assert!(result.contains("📁 subdir"));
    }

    #[tokio::test]
    async fn test_list_dir_empty() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ListDirTool::new(None);
        let result = tool
            .execute(make_params(&[("path", dir.path().to_str().unwrap())]))
            .await
            .unwrap();
        assert_eq!(result, "(empty directory)");
    }

    #[tokio::test]
    async fn test_list_dir_not_a_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file.txt");
        std::fs::write(&file, "").unwrap();

        let tool = ListDirTool::new(None);
        let result = tool
            .execute(make_params(&[("path", file.to_str().unwrap())]))
            .await;
        assert!(result.is_err());
    }

    // ── Tool definitions ──

    #[test]
    fn test_tool_definitions() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(ReadFileTool::new(None)),
            Box::new(WriteFileTool::new(None)),
            Box::new(EditFileTool::new(None)),
            Box::new(ListDirTool::new(None)),
        ];
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, vec!["read_file", "write_file", "edit_file", "list_dir"]);

        // Each produces a valid ToolDefinition
        for tool in &tools {
            let def = tool.to_definition();
            assert_eq!(def.tool_type, "function");
            assert!(!def.function.description.is_empty());
        }
    }
}
