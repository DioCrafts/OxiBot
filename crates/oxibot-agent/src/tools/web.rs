//! Web tools — search (Brave API) and fetch (HTTP content extraction).
//!
//! Port of nanobot's `agent/tools/web.py`.

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use tracing::debug;

use super::base::{optional_i64, require_string, Tool};

/// User-Agent header.
const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/537.36 (KHTML, like Gecko)";

/// Max chars for fetched content.
const DEFAULT_MAX_CHARS: usize = 50_000;

/// Max search results.
const DEFAULT_MAX_RESULTS: usize = 5;

// ─────────────────────────────────────────────
// WebSearchTool (Brave API)
// ─────────────────────────────────────────────

/// Searches the web using the Brave Search API.
pub struct WebSearchTool {
    api_key: Option<String>,
    client: Client,
}

impl WebSearchTool {
    /// Create a new web search tool.
    ///
    /// `api_key` can be `None`; it will fall back to `BRAVE_API_KEY` env var.
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            client: Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .unwrap_or_default(),
        }
    }

    fn resolve_api_key(&self) -> Option<String> {
        self.api_key
            .clone()
            .or_else(|| std::env::var("BRAVE_API_KEY").ok())
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web using Brave Search API. Returns a numbered list of results with titles, URLs, and descriptions."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results (1-10, default 5)",
                    "minimum": 1,
                    "maximum": 10
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: HashMap<String, Value>) -> anyhow::Result<String> {
        let query = require_string(&params, "query")?;
        let count = optional_i64(&params, "count").unwrap_or(DEFAULT_MAX_RESULTS as i64) as usize;
        let count = count.min(10).max(1);

        let api_key = self
            .resolve_api_key()
            .ok_or_else(|| anyhow::anyhow!("No Brave API key configured (set BRAVE_API_KEY env var)"))?;

        debug!(query = %query, count = count, "searching web");

        let resp = self
            .client
            .get("https://api.search.brave.com/res/v1/web/search")
            .header("X-Subscription-Token", &api_key)
            .query(&[("q", &query), ("count", &count.to_string())])
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Brave API request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Brave API returned {status}: {body}");
        }

        let body: Value = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse Brave response: {e}"))?;

        let results = body["web"]["results"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        if results.is_empty() {
            return Ok("No results found.".into());
        }

        let mut output = Vec::new();
        for (i, r) in results.iter().enumerate() {
            let title = r["title"].as_str().unwrap_or("(no title)");
            let url = r["url"].as_str().unwrap_or("");
            let desc = r["description"].as_str().unwrap_or("");
            output.push(format!("{}. {}\n   {}\n   {}", i + 1, title, url, desc));
        }

        Ok(output.join("\n\n"))
    }
}

// ─────────────────────────────────────────────
// WebFetchTool
// ─────────────────────────────────────────────

/// Fetches and extracts content from a web page.
pub struct WebFetchTool {
    client: Client,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent(USER_AGENT)
                .redirect(reqwest::redirect::Policy::limited(5))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch and extract the main text content from a web page URL. \
         Supports HTML (converted to text) and JSON."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "maxChars": {
                    "type": "integer",
                    "description": "Maximum characters to return (default 50000)",
                    "minimum": 100
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, params: HashMap<String, Value>) -> anyhow::Result<String> {
        let url = require_string(&params, "url")?;
        let max_chars = optional_i64(&params, "maxChars").unwrap_or(DEFAULT_MAX_CHARS as i64) as usize;
        let max_chars = max_chars.max(100);

        // Validate URL
        if !url.starts_with("http://") && !url.starts_with("https://") {
            anyhow::bail!("Invalid URL: must start with http:// or https://");
        }

        debug!(url = %url, "fetching web page");

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("HTTP request failed: {e}"))?;

        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = resp
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read response body: {e}"))?;

        // Choose extraction method
        let (text, extractor) = if content_type.contains("json") {
            // Pretty-print JSON
            match serde_json::from_str::<Value>(&body) {
                Ok(v) => (
                    serde_json::to_string_pretty(&v).unwrap_or(body),
                    "json",
                ),
                Err(_) => (body, "raw"),
            }
        } else if content_type.contains("html") || body.trim_start().starts_with('<') {
            // Strip HTML tags → plain text
            (strip_html_tags(&body), "text")
        } else {
            (body, "raw")
        };

        // Truncate
        let truncated = text.len() > max_chars;
        let text = if truncated {
            text[..max_chars].to_string()
        } else {
            text
        };

        let result = json!({
            "url": url,
            "finalUrl": final_url,
            "status": status,
            "extractor": extractor,
            "truncated": truncated,
            "length": text.len(),
            "text": text,
        });

        Ok(serde_json::to_string_pretty(&result).unwrap_or_default())
    }
}

// ─────────────────────────────────────────────
// HTML helpers
// ─────────────────────────────────────────────

/// Remove HTML tags, scripts, and styles, then collapse whitespace.
///
/// Simple regex-free approach suitable for LLM consumption.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut tag_name = String::new();
    let mut collecting_tag_name = false;

    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
            collecting_tag_name = true;
            tag_name.clear();
            continue;
        }
        if ch == '>' {
            in_tag = false;
            collecting_tag_name = false;
            let lower = tag_name.to_lowercase();
            if lower == "script" {
                in_script = true;
            } else if lower == "/script" {
                in_script = false;
            } else if lower == "style" {
                in_style = true;
            } else if lower == "/style" {
                in_style = false;
            } else if lower == "br" || lower == "br/" || lower == "br /" {
                result.push('\n');
            } else if lower == "p" || lower == "/p" || lower == "div" || lower == "/div" {
                result.push('\n');
            }
            continue;
        }
        if in_tag {
            if collecting_tag_name && (ch.is_alphanumeric() || ch == '/') {
                tag_name.push(ch);
            } else {
                collecting_tag_name = false;
            }
            continue;
        }
        if in_script || in_style {
            continue;
        }
        // Decode common entities
        result.push(ch);
    }

    // Decode a few HTML entities
    let result = result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // Collapse whitespace
    let mut prev_space = false;
    let collapsed: String = result
        .chars()
        .filter_map(|c| {
            if c == '\n' {
                prev_space = false;
                Some('\n')
            } else if c.is_whitespace() {
                if prev_space {
                    None
                } else {
                    prev_space = true;
                    Some(' ')
                }
            } else {
                prev_space = false;
                Some(c)
            }
        })
        .collect();

    // Collapse multiple newlines
    let mut final_text = String::with_capacity(collapsed.len());
    let mut prev_newline = false;
    for ch in collapsed.chars() {
        if ch == '\n' {
            if !prev_newline {
                final_text.push('\n');
            }
            prev_newline = true;
        } else {
            prev_newline = false;
            final_text.push(ch);
        }
    }

    final_text.trim().to_string()
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html_basic() {
        let html = "<html><body><h1>Title</h1><p>Hello <b>world</b></p></body></html>";
        let text = strip_html_tags(html);
        assert!(text.contains("Title"));
        assert!(text.contains("Hello world"));
        assert!(!text.contains("<"));
    }

    #[test]
    fn test_strip_html_script() {
        let html = "<p>Before</p><script>alert('xss');</script><p>After</p>";
        let text = strip_html_tags(html);
        assert!(text.contains("Before"));
        assert!(text.contains("After"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn test_strip_html_style() {
        let html = "<style>body { color: red; }</style><p>Content</p>";
        let text = strip_html_tags(html);
        assert!(text.contains("Content"));
        assert!(!text.contains("color"));
    }

    #[test]
    fn test_strip_html_entities() {
        let html = "<p>A &amp; B &lt; C &gt; D</p>";
        let text = strip_html_tags(html);
        assert!(text.contains("A & B < C > D"));
    }

    #[test]
    fn test_strip_html_br() {
        let html = "Line1<br>Line2<br/>Line3";
        let text = strip_html_tags(html);
        assert!(text.contains("Line1\nLine2\nLine3"));
    }

    #[test]
    fn test_web_search_definition() {
        let tool = WebSearchTool::new(None);
        let def = tool.to_definition();
        assert_eq!(def.function.name, "web_search");
    }

    #[test]
    fn test_web_fetch_definition() {
        let tool = WebFetchTool::new();
        let def = tool.to_definition();
        assert_eq!(def.function.name, "web_fetch");
    }

    #[tokio::test]
    async fn test_web_fetch_invalid_url() {
        let tool = WebFetchTool::new();
        let mut params = HashMap::new();
        params.insert("url".into(), json!("not-a-url"));
        let result = tool.execute(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid URL"));
    }

    #[tokio::test]
    async fn test_web_search_no_api_key() {
        // Unset the env var to ensure no key
        std::env::remove_var("BRAVE_API_KEY");
        let tool = WebSearchTool::new(None);
        let mut params = HashMap::new();
        params.insert("query".into(), json!("test"));
        let result = tool.execute(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("API key"));
    }
}
