//! Skills loader — discovers, loads, and filters skill files.
//!
//! Port of nanobot's `agent/skills.py`.
//!
//! # Architecture
//!
//! Skills are **Markdown files** (`SKILL.md`) that teach the agent how to
//! combine existing tools for specific domains (GitHub, weather, tmux, etc.).
//! They do **not** register new tools.
//!
//! ## Two-tier loading
//!
//! 1. **Always-on skills** (`always: true` in metadata) — full content injected
//!    into every system prompt.
//! 2. **On-demand skills** — only an XML summary (name, description, path,
//!    availability) is injected. The LLM uses `read_file` to load the full
//!    `SKILL.md` when it decides a skill is relevant.
//!
//! ## Discovery order
//!
//! 1. `workspace/skills/<name>/SKILL.md` (user custom — highest priority)
//! 2. Built-in skills bundled with oxibot (lower priority, overridden by name)
//!
//! ## SKILL.md format
//!
//! ```text
//! ---
//! name: github
//! description: "Interact with GitHub using the gh CLI"
//! metadata: {"nanobot":{"requires":{"bins":["gh"]},"always":false}}
//! ---
//!
//! # GitHub Skill
//!
//! Use the `exec` tool to run `gh` commands ...
//! ```

use std::path::{Path, PathBuf};

use tracing::debug;

// ─────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────

/// Where a skill was discovered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillSource {
    /// User's workspace `skills/` directory.
    Workspace,
    /// Built-in with oxibot.
    Builtin,
}

/// Metadata about a discovered skill.
#[derive(Clone, Debug)]
pub struct SkillInfo {
    /// Skill name (directory name).
    pub name: String,
    /// Path to the `SKILL.md` file.
    pub path: PathBuf,
    /// Where the skill was found.
    pub source: SkillSource,
}

/// Parsed requirements for a skill.
#[derive(Clone, Debug, Default)]
pub struct SkillRequires {
    /// CLI binaries that must be on PATH.
    pub bins: Vec<String>,
    /// Environment variables that must be set.
    pub env: Vec<String>,
}

/// Parsed nanobot metadata block from the frontmatter `metadata` JSON field.
#[derive(Clone, Debug, Default)]
pub struct SkillMeta {
    /// If true, full content is always injected into the system prompt.
    pub always: bool,
    /// Requirements for this skill to be available.
    pub requires: SkillRequires,
    /// Description (from frontmatter top-level, not metadata JSON).
    pub description: Option<String>,
}

// ─────────────────────────────────────────────
// SkillsLoader
// ─────────────────────────────────────────────

/// Discovers and loads skill files from workspace and built-in directories.
pub struct SkillsLoader {
    /// User workspace root.
    workspace_skills: PathBuf,
    /// Built-in skills directory.
    builtin_skills: Option<PathBuf>,
}

impl SkillsLoader {
    /// Create a new skills loader.
    ///
    /// - `workspace` — the agent workspace root (contains `skills/` subdirectory)
    /// - `builtin_skills` — optional path to built-in skills directory
    pub fn new(workspace: &Path, builtin_skills: Option<PathBuf>) -> Self {
        Self {
            workspace_skills: workspace.join("skills"),
            builtin_skills,
        }
    }

    // ────────────── Discovery ──────────────

    /// List all discovered skills.
    ///
    /// If `filter_unavailable` is true, skills with unmet requirements
    /// (missing CLI binaries or env vars) are excluded.
    pub fn list_skills(&self, filter_unavailable: bool) -> Vec<SkillInfo> {
        let mut skills = Vec::new();

        // 1. Workspace skills (highest priority)
        if self.workspace_skills.is_dir() {
            scan_skill_dirs(&self.workspace_skills, SkillSource::Workspace, &mut skills);
        }

        // 2. Built-in skills (skip if name already found in workspace)
        if let Some(builtin) = &self.builtin_skills {
            if builtin.is_dir() {
                let existing: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
                let mut builtin_skills = Vec::new();
                scan_skill_dirs(builtin, SkillSource::Builtin, &mut builtin_skills);
                for skill in builtin_skills {
                    if !existing.contains(&skill.name) {
                        skills.push(skill);
                    }
                }
            }
        }

        if filter_unavailable {
            skills.retain(|s| {
                let meta = self.get_skill_meta(&s.name);
                check_requirements(&meta.requires)
            });
        }

        skills
    }

    // ────────────── Loading ──────────────

    /// Load the raw content of a skill by name.
    ///
    /// Looks in workspace first, then built-in.
    pub fn load_skill(&self, name: &str) -> Option<String> {
        let ws_path = self.workspace_skills.join(name).join("SKILL.md");
        if ws_path.is_file() {
            return std::fs::read_to_string(&ws_path).ok();
        }

        if let Some(builtin) = &self.builtin_skills {
            let bi_path = builtin.join(name).join("SKILL.md");
            if bi_path.is_file() {
                return std::fs::read_to_string(&bi_path).ok();
            }
        }

        None
    }

    /// Load full content of specific skills, stripped of frontmatter,
    /// for injection into the system prompt (always-on skills).
    pub fn load_skills_for_context(&self, names: &[String]) -> String {
        let parts: Vec<String> = names
            .iter()
            .filter_map(|name| {
                let content = self.load_skill(name)?;
                let body = strip_frontmatter(&content);
                if body.is_empty() {
                    return None;
                }
                Some(format!("### Skill: {name}\n\n{body}"))
            })
            .collect();

        parts.join("\n\n---\n\n")
    }

    /// Build an XML summary of all skills for the system prompt.
    ///
    /// The LLM uses this to decide which skills to load on demand via `read_file`.
    pub fn build_skills_summary(&self) -> String {
        let all = self.list_skills(false);
        if all.is_empty() {
            return String::new();
        }

        let mut lines = vec!["<skills>".to_string()];

        for skill in &all {
            let meta = self.get_skill_meta(&skill.name);
            let available = check_requirements(&meta.requires);
            let desc = meta
                .description
                .as_deref()
                .unwrap_or(&skill.name);

            lines.push(format!(
                "  <skill available=\"{}\">",
                if available { "true" } else { "false" }
            ));
            lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
            lines.push(format!("    <description>{}</description>", escape_xml(desc)));
            lines.push(format!("    <location>{}</location>", skill.path.display()));

            if !available {
                let missing = get_missing_requirements(&meta.requires);
                if !missing.is_empty() {
                    lines.push(format!("    <requires>{}</requires>", escape_xml(&missing)));
                }
            }

            lines.push("  </skill>".to_string());
        }

        lines.push("</skills>".to_string());
        lines.join("\n")
    }

    /// Get names of skills that should always be injected (full body).
    pub fn get_always_skills(&self) -> Vec<String> {
        self.list_skills(true)
            .iter()
            .filter(|s| {
                let meta = self.get_skill_meta(&s.name);
                meta.always
            })
            .map(|s| s.name.clone())
            .collect()
    }

    // ────────────── Metadata ──────────────

    /// Parse frontmatter metadata for a skill.
    pub fn get_skill_meta(&self, name: &str) -> SkillMeta {
        let content = match self.load_skill(name) {
            Some(c) => c,
            None => return SkillMeta::default(),
        };

        let frontmatter = match parse_frontmatter(&content) {
            Some(fm) => fm,
            None => return SkillMeta::default(),
        };

        // Top-level description
        let description = frontmatter
            .iter()
            .find(|(k, _)| k == "description")
            .map(|(_, v)| v.trim_matches('"').trim_matches('\'').to_string());

        // Top-level `always`
        let always_top = frontmatter
            .iter()
            .find(|(k, _)| k == "always")
            .map(|(_, v)| v == "true")
            .unwrap_or(false);

        // Parse the `metadata` field (JSON string containing nanobot config)
        let metadata_json = frontmatter
            .iter()
            .find(|(k, _)| k == "metadata")
            .map(|(_, v)| v.as_str())
            .unwrap_or("");

        let (nanobot_always, requires) = parse_nanobot_metadata(metadata_json);

        SkillMeta {
            always: always_top || nanobot_always,
            requires,
            description,
        }
    }
}

// ─────────────────────────────────────────────
// Helper functions
// ─────────────────────────────────────────────

/// Scan a directory for skill subdirectories containing `SKILL.md`.
fn scan_skill_dirs(dir: &Path, source: SkillSource, out: &mut Vec<SkillInfo>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let skill_file = path.join("SKILL.md");
            if skill_file.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    debug!(name, source = ?source, "discovered skill");
                    out.push(SkillInfo {
                        name: name.to_string(),
                        path: skill_file,
                        source: source.clone(),
                    });
                }
            }
        }
    }
}

/// Parse YAML-like frontmatter (between `---` delimiters) into key-value pairs.
///
/// Uses naive line-by-line parsing (matching nanobot's approach).
fn parse_frontmatter(content: &str) -> Option<Vec<(String, String)>> {
    if !content.starts_with("---") {
        return None;
    }

    let after_first = &content[3..];
    let end = after_first.find("\n---")?;
    let block = &after_first[..end];

    let mut pairs = Vec::new();
    for line in block.lines() {
        let line = line.trim();
        if let Some(idx) = line.find(':') {
            let key = line[..idx].trim().to_string();
            let value = line[idx + 1..].trim().to_string();
            if !key.is_empty() {
                pairs.push((key, value));
            }
        }
    }

    Some(pairs)
}

/// Parse the `metadata` JSON field for nanobot-specific config.
///
/// Expected format: `{"nanobot":{"always":true,"requires":{"bins":["gh"],"env":["TOKEN"]}}}`
fn parse_nanobot_metadata(raw: &str) -> (bool, SkillRequires) {
    let value: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return (false, SkillRequires::default()),
    };

    let nanobot = match value.get("nanobot") {
        Some(n) => n,
        None => return (false, SkillRequires::default()),
    };

    let always = nanobot
        .get("always")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let requires = match nanobot.get("requires") {
        Some(r) => {
            let bins = r
                .get("bins")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let env = r
                .get("env")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            SkillRequires { bins, env }
        }
        None => SkillRequires::default(),
    };

    (always, requires)
}

/// Strip YAML frontmatter from markdown content.
fn strip_frontmatter(content: &str) -> &str {
    if !content.starts_with("---") {
        return content;
    }
    let after_first = &content[3..];
    match after_first.find("\n---") {
        Some(end) => {
            let rest = &after_first[end + 4..]; // skip "\n---"
            rest.trim_start_matches('\n')
        }
        None => content,
    }
}

/// Escape XML special characters.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Check if all requirements are met.
fn check_requirements(requires: &SkillRequires) -> bool {
    for bin in &requires.bins {
        if !is_binary_available(bin) {
            return false;
        }
    }
    for env_var in &requires.env {
        if std::env::var(env_var).is_err() {
            return false;
        }
    }
    true
}

/// Get a human-readable list of missing requirements.
fn get_missing_requirements(requires: &SkillRequires) -> String {
    let mut missing = Vec::new();

    for bin in &requires.bins {
        if !is_binary_available(bin) {
            missing.push(format!("CLI: {bin}"));
        }
    }
    for env_var in &requires.env {
        if std::env::var(env_var).is_err() {
            missing.push(format!("ENV: {env_var}"));
        }
    }

    missing.join(", ")
}

/// Check if a binary is available on the system PATH.
fn is_binary_available(name: &str) -> bool {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a temp skill directory with a SKILL.md file.
    fn create_skill(base: &Path, name: &str, content: &str) {
        let skill_dir = base.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    // ────────────── Frontmatter parsing ──────────────

    #[test]
    fn parse_frontmatter_valid() {
        let content = "---\nname: github\ndescription: \"GitHub CLI\"\n---\n\n# Body";
        let fm = parse_frontmatter(content).unwrap();
        assert_eq!(fm.len(), 2);
        assert_eq!(fm[0], ("name".into(), "github".into()));
        assert_eq!(fm[1], ("description".into(), "\"GitHub CLI\"".into()));
    }

    #[test]
    fn parse_frontmatter_none_when_no_delimiters() {
        assert!(parse_frontmatter("# Just markdown").is_none());
    }

    #[test]
    fn parse_frontmatter_with_metadata_json() {
        let content = "---\nname: test\nmetadata: {\"nanobot\":{\"always\":true}}\n---\n\nBody";
        let fm = parse_frontmatter(content).unwrap();
        let meta_val = fm.iter().find(|(k, _)| k == "metadata").unwrap();
        assert!(meta_val.1.contains("nanobot"));
    }

    // ────────────── Nanobot metadata ──────────────

    #[test]
    fn parse_nanobot_metadata_with_requires() {
        let json = r#"{"nanobot":{"requires":{"bins":["gh","git"],"env":["GITHUB_TOKEN"]},"always":true}}"#;
        let (always, req) = parse_nanobot_metadata(json);
        assert!(always);
        assert_eq!(req.bins, vec!["gh", "git"]);
        assert_eq!(req.env, vec!["GITHUB_TOKEN"]);
    }

    #[test]
    fn parse_nanobot_metadata_empty() {
        let (always, req) = parse_nanobot_metadata("");
        assert!(!always);
        assert!(req.bins.is_empty());
    }

    #[test]
    fn parse_nanobot_metadata_no_nanobot_key() {
        let (always, req) = parse_nanobot_metadata(r#"{"other":"value"}"#);
        assert!(!always);
        assert!(req.bins.is_empty());
    }

    // ────────────── Strip frontmatter ──────────────

    #[test]
    fn strip_frontmatter_removes_header() {
        let content = "---\nname: test\n---\n\n# Body here";
        assert_eq!(strip_frontmatter(content), "# Body here");
    }

    #[test]
    fn strip_frontmatter_no_header() {
        let content = "# Just body";
        assert_eq!(strip_frontmatter(content), "# Just body");
    }

    // ────────────── XML escaping ──────────────

    #[test]
    fn escape_xml_special_chars() {
        assert_eq!(escape_xml("a<b>c&d\"e"), "a&lt;b&gt;c&amp;d&quot;e");
    }

    // ────────────── Requirements checking ──────────────

    #[test]
    fn check_requirements_empty() {
        assert!(check_requirements(&SkillRequires::default()));
    }

    #[test]
    fn check_requirements_missing_bin() {
        let req = SkillRequires {
            bins: vec!["__nonexistent_binary_xyz__".into()],
            env: vec![],
        };
        assert!(!check_requirements(&req));
    }

    #[test]
    fn check_requirements_missing_env() {
        let req = SkillRequires {
            bins: vec![],
            env: vec!["__NONEXISTENT_ENV_VAR_XYZ__".into()],
        };
        assert!(!check_requirements(&req));
    }

    #[test]
    fn check_requirements_bin_available() {
        // `ls` should be available on any Unix system
        let req = SkillRequires {
            bins: vec!["ls".into()],
            env: vec![],
        };
        assert!(check_requirements(&req));
    }

    #[test]
    fn get_missing_requirements_report() {
        let req = SkillRequires {
            bins: vec!["__no_bin__".into()],
            env: vec!["__NO_ENV__".into()],
        };
        let report = get_missing_requirements(&req);
        assert!(report.contains("CLI: __no_bin__"));
        assert!(report.contains("ENV: __NO_ENV__"));
    }

    // ────────────── SkillsLoader ──────────────

    #[test]
    fn list_skills_empty_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let loader = SkillsLoader::new(dir.path(), None);
        assert!(loader.list_skills(false).is_empty());
    }

    #[test]
    fn list_skills_finds_workspace_skills() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        create_skill(&ws.join("skills"), "my-skill", "---\nname: my-skill\n---\n\n# Hello");

        let loader = SkillsLoader::new(ws, None);
        let skills = loader.list_skills(false);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
        assert_eq!(skills[0].source, SkillSource::Workspace);
    }

    #[test]
    fn list_skills_finds_builtin_skills() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("workspace");
        let builtin = dir.path().join("builtin");
        fs::create_dir_all(&ws).unwrap();
        create_skill(&builtin, "github", "---\nname: github\n---\n\n# GitHub");

        let loader = SkillsLoader::new(&ws, Some(builtin));
        let skills = loader.list_skills(false);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "github");
        assert_eq!(skills[0].source, SkillSource::Builtin);
    }

    #[test]
    fn workspace_skills_override_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("workspace");
        let builtin = dir.path().join("builtin");

        create_skill(&ws.join("skills"), "github", "---\nname: github\n---\n\n# Custom");
        create_skill(&builtin, "github", "---\nname: github\n---\n\n# Builtin");

        let loader = SkillsLoader::new(&ws, Some(builtin));
        let skills = loader.list_skills(false);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source, SkillSource::Workspace);
    }

    #[test]
    fn load_skill_returns_content() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        create_skill(&ws.join("skills"), "test", "---\nname: test\n---\n\nBody line");

        let loader = SkillsLoader::new(ws, None);
        let content = loader.load_skill("test").unwrap();
        assert!(content.contains("Body line"));
    }

    #[test]
    fn load_skill_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let loader = SkillsLoader::new(dir.path(), None);
        assert!(loader.load_skill("nonexistent").is_none());
    }

    #[test]
    fn load_skills_for_context_strips_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        create_skill(&ws.join("skills"), "alpha", "---\nname: alpha\n---\n\n# Alpha Body");

        let loader = SkillsLoader::new(ws, None);
        let ctx = loader.load_skills_for_context(&["alpha".into()]);
        assert!(ctx.contains("### Skill: alpha"));
        assert!(ctx.contains("# Alpha Body"));
        assert!(!ctx.contains("---"));
    }

    #[test]
    fn build_skills_summary_xml() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        create_skill(
            &ws.join("skills"),
            "weather",
            "---\nname: weather\ndescription: \"Check the weather\"\n---\n\n# Weather",
        );

        let loader = SkillsLoader::new(ws, None);
        let summary = loader.build_skills_summary();
        assert!(summary.contains("<skills>"));
        assert!(summary.contains("<name>weather</name>"));
        assert!(summary.contains("<description>Check the weather</description>"));
        assert!(summary.contains("available=\"true\""));
        assert!(summary.contains("</skills>"));
    }

    #[test]
    fn build_skills_summary_unavailable_skill() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        create_skill(
            &ws.join("skills"),
            "fancy",
            "---\nname: fancy\ndescription: needs binary\nmetadata: {\"nanobot\":{\"requires\":{\"bins\":[\"__nonexistent__\"]}}}\n---\n\n# Fancy",
        );

        let loader = SkillsLoader::new(ws, None);
        let summary = loader.build_skills_summary();
        assert!(summary.contains("available=\"false\""));
        assert!(summary.contains("<requires>"));
    }

    #[test]
    fn get_always_skills_returns_matching() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        create_skill(
            &ws.join("skills"),
            "always-on",
            "---\nname: always-on\nmetadata: {\"nanobot\":{\"always\":true}}\n---\n\n# Always",
        );
        create_skill(
            &ws.join("skills"),
            "on-demand",
            "---\nname: on-demand\n---\n\n# On demand",
        );

        let loader = SkillsLoader::new(ws, None);
        let always = loader.get_always_skills();
        assert_eq!(always, vec!["always-on"]);
    }

    #[test]
    fn get_always_skills_top_level_flag() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        create_skill(
            &ws.join("skills"),
            "top-always",
            "---\nname: top-always\nalways: true\n---\n\n# Always via top-level",
        );

        let loader = SkillsLoader::new(ws, None);
        let always = loader.get_always_skills();
        assert_eq!(always, vec!["top-always"]);
    }

    #[test]
    fn get_skill_meta_full() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        create_skill(
            &ws.join("skills"),
            "full",
            "---\nname: full\ndescription: \"Full skill\"\nmetadata: {\"nanobot\":{\"always\":true,\"requires\":{\"bins\":[\"curl\"],\"env\":[\"API_KEY\"]}}}\n---\n\n# Full",
        );

        let loader = SkillsLoader::new(ws, None);
        let meta = loader.get_skill_meta("full");
        assert!(meta.always);
        assert_eq!(meta.description.as_deref(), Some("Full skill"));
        assert_eq!(meta.requires.bins, vec!["curl"]);
        assert_eq!(meta.requires.env, vec!["API_KEY"]);
    }

    #[test]
    fn filter_unavailable_excludes_missing_bins() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        create_skill(
            &ws.join("skills"),
            "available",
            "---\nname: available\n---\n\n# OK",
        );
        create_skill(
            &ws.join("skills"),
            "unavailable",
            "---\nname: unavailable\nmetadata: {\"nanobot\":{\"requires\":{\"bins\":[\"__nope__\"]}}}\n---\n\n# Nope",
        );

        let loader = SkillsLoader::new(ws, None);
        let all = loader.list_skills(false);
        assert_eq!(all.len(), 2);

        let filtered = loader.list_skills(true);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "available");
    }
}
