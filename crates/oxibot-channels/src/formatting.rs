//! Markdown → Telegram HTML converter.
//!
//! Telegram supports a subset of HTML for formatted messages.
//! This module converts standard Markdown (as produced by LLMs)
//! to Telegram-compatible HTML.
//!
//! Supported conversions:
//! - Code blocks (```) → `<pre><code>...</code></pre>`
//! - Inline code (`) → `<code>...</code>`
//! - Bold (**) → `<b>...</b>`
//! - Italic (_) → `<i>...</i>`
//! - Strikethrough (~~) → `<s>...</s>`
//! - Links [text](url) → `<a href="url">text</a>`
//! - Headers (# ...) → stripped to plain text
//! - Blockquotes (> ...) → stripped to plain text
//! - Bullets (- / *) → `•`

use regex::Regex;

/// Convert Markdown text to Telegram-compatible HTML.
///
/// If conversion fails or the result would be invalid,
/// the caller should fall back to plain text.
pub fn markdown_to_telegram_html(text: &str) -> String {
    // 1. Extract and protect code blocks
    let mut code_blocks: Vec<String> = Vec::new();
    let re_code_block = Regex::new(r"(?s)```(?:\w+)?\n?(.*?)```").unwrap();
    let text = re_code_block.replace_all(text, |caps: &regex::Captures| {
        let idx = code_blocks.len();
        code_blocks.push(caps[1].to_string());
        format!("\x00CB{idx}\x00")
    });

    // 2. Extract and protect inline code
    let mut inline_codes: Vec<String> = Vec::new();
    let re_inline = Regex::new(r"`([^`]+)`").unwrap();
    let text = re_inline.replace_all(&text, |caps: &regex::Captures| {
        let idx = inline_codes.len();
        inline_codes.push(caps[1].to_string());
        format!("\x00IC{idx}\x00")
    });

    // 3. Strip headers (# Title → Title)
    let re_headers = Regex::new(r"(?m)^#{1,6}\s+(.+)$").unwrap();
    let text = re_headers.replace_all(&text, "$1");

    // 4. Strip blockquotes (> text → text)
    let re_blockquote = Regex::new(r"(?m)^>\s?(.*)$").unwrap();
    let text = re_blockquote.replace_all(&text, "$1");

    // 5. Escape HTML entities
    let text = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    // 6. Links [text](url) → <a href="url">text</a>
    let re_links = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();
    let text = re_links.replace_all(&text, r#"<a href="$2">$1</a>"#);

    // 7. Bold **text** and __text__
    let re_bold_star = Regex::new(r"\*\*(.+?)\*\*").unwrap();
    let text = re_bold_star.replace_all(&text, "<b>$1</b>");
    let re_bold_under = Regex::new(r"__(.+?)__").unwrap();
    let text = re_bold_under.replace_all(&text, "<b>$1</b>");

    // 8. Italic _text_ (with word-boundary guards to avoid matching snake_case)
    //    Rust regex doesn't support lookbehind, so we capture surrounding context.
    //    Match _text_ only when preceded by start-of-string/non-word or followed by
    //    end-of-string/non-word.
    let re_italic = Regex::new(r"(^|[^a-zA-Z0-9_])_([^_]+?)_($|[^a-zA-Z0-9_])").unwrap();
    let text = re_italic.replace_all(&text, "$1<i>$2</i>$3");

    // 9. Strikethrough ~~text~~
    let re_strike = Regex::new(r"~~(.+?)~~").unwrap();
    let text = re_strike.replace_all(&text, "<s>$1</s>");

    // 10. Bullets - item / * item → • item
    let re_bullet = Regex::new(r"(?m)^[\s]*[-*]\s+").unwrap();
    let text = re_bullet.replace_all(&text, "• ");

    // 11. Restore inline code → <code>escaped</code>
    let mut text = text.to_string();
    for (idx, code) in inline_codes.iter().enumerate() {
        let escaped = code
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        text = text.replace(
            &format!("\x00IC{idx}\x00"),
            &format!("<code>{escaped}</code>"),
        );
    }

    // 12. Restore code blocks → <pre><code>escaped</code></pre>
    for (idx, code) in code_blocks.iter().enumerate() {
        let escaped = code
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        text = text.replace(
            &format!("\x00CB{idx}\x00"),
            &format!("<pre><code>{escaped}</code></pre>"),
        );
    }

    text
}

/// Split a message into chunks that fit within Telegram's 4096-char limit.
///
/// Tries to split at newline boundaries to avoid breaking mid-sentence.
pub fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Try to split at a newline near the limit
        let split_at = remaining[..max_len]
            .rfind('\n')
            .unwrap_or(max_len);

        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk.to_string());

        // Skip the newline character if we split there
        remaining = if rest.starts_with('\n') {
            &rest[1..]
        } else {
            rest
        };
    }

    chunks
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bold() {
        assert_eq!(
            markdown_to_telegram_html("Hello **world**!"),
            "Hello <b>world</b>!"
        );
    }

    #[test]
    fn test_italic() {
        assert_eq!(
            markdown_to_telegram_html("Hello _world_!"),
            "Hello <i>world</i>!"
        );
    }

    #[test]
    fn test_italic_no_snake_case() {
        // snake_case should NOT be italicized
        let result = markdown_to_telegram_html("use my_var_name here");
        assert_eq!(result, "use my_var_name here");
    }

    #[test]
    fn test_strikethrough() {
        assert_eq!(
            markdown_to_telegram_html("~~deleted~~"),
            "<s>deleted</s>"
        );
    }

    #[test]
    fn test_inline_code() {
        assert_eq!(
            markdown_to_telegram_html("Use `println!` macro"),
            "Use <code>println!</code> macro"
        );
    }

    #[test]
    fn test_code_block() {
        let input = "```rust\nfn main() {}\n```";
        let result = markdown_to_telegram_html(input);
        assert!(result.contains("<pre><code>fn main() {}\n</code></pre>"));
    }

    #[test]
    fn test_link() {
        assert_eq!(
            markdown_to_telegram_html("[Rust](https://rust-lang.org)"),
            r#"<a href="https://rust-lang.org">Rust</a>"#
        );
    }

    #[test]
    fn test_header_stripped() {
        assert_eq!(
            markdown_to_telegram_html("# Hello World"),
            "Hello World"
        );
    }

    #[test]
    fn test_h3_stripped() {
        assert_eq!(
            markdown_to_telegram_html("### Deep Header"),
            "Deep Header"
        );
    }

    #[test]
    fn test_blockquote_stripped() {
        assert_eq!(
            markdown_to_telegram_html("> quoted text"),
            "quoted text"
        );
    }

    #[test]
    fn test_bullet_conversion() {
        let input = "- item one\n- item two\n* item three";
        let result = markdown_to_telegram_html(input);
        assert!(result.contains("• item one"));
        assert!(result.contains("• item two"));
        assert!(result.contains("• item three"));
    }

    #[test]
    fn test_html_escaping() {
        assert_eq!(
            markdown_to_telegram_html("x < y && z > w"),
            "x &lt; y &amp;&amp; z &gt; w"
        );
    }

    #[test]
    fn test_code_block_preserves_html() {
        let input = "```\n<div>&amp;</div>\n```";
        let result = markdown_to_telegram_html(input);
        assert!(result.contains("&lt;div&gt;&amp;amp;&lt;/div&gt;"));
    }

    #[test]
    fn test_complex_message() {
        let input = "# Title\n\nHello **bold** and _italic_.\n\n```\ncode here\n```\n\nUse `var`.\n\n- one\n- two";
        let result = markdown_to_telegram_html(input);
        assert!(result.contains("<b>bold</b>"));
        assert!(result.contains("<i>italic</i>"));
        assert!(result.contains("<pre><code>code here\n</code></pre>"));
        assert!(result.contains("<code>var</code>"));
        assert!(result.contains("• one"));
    }

    #[test]
    fn test_split_message_short() {
        let chunks = split_message("short", 4096);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "short");
    }

    #[test]
    fn test_split_message_at_newline() {
        let text = format!("{}\n{}", "a".repeat(50), "b".repeat(50));
        let chunks = split_message(&text, 60);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "a".repeat(50));
        assert_eq!(chunks[1], "b".repeat(50));
    }

    #[test]
    fn test_split_message_no_newline() {
        let text = "a".repeat(100);
        let chunks = split_message(&text, 60);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 60);
        assert_eq!(chunks[1].len(), 40);
    }

    #[test]
    fn test_split_message_empty() {
        let chunks = split_message("", 4096);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "");
    }
}
