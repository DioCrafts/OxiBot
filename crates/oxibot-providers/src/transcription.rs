//! Voice transcription providers — speech-to-text via Whisper APIs.
//!
//! Port of nanobot's `providers/transcription.py`.
//!
//! Currently supports Groq's Whisper API (fast, free tier available).
//! Any OpenAI-compatible `/v1/audio/transcriptions` endpoint will work.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use tracing::{debug, error, warn};

// ─────────────────────────────────────────────
// Trait
// ─────────────────────────────────────────────

/// Trait for speech-to-text transcription providers.
#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    /// Transcribe an audio file to text.
    ///
    /// Returns the transcribed text, or empty string on failure.
    async fn transcribe(&self, file_path: &Path) -> anyhow::Result<String>;

    /// Display name for logging.
    fn display_name(&self) -> &str;
}

// ─────────────────────────────────────────────
// Groq Whisper
// ─────────────────────────────────────────────

/// Groq-based transcription using their Whisper API.
///
/// Groq offers extremely fast transcription with a generous free tier.
/// API is OpenAI-compatible (`/openai/v1/audio/transcriptions`).
pub struct GroqTranscriber {
    api_key: String,
    api_url: String,
    model: String,
    client: reqwest::Client,
}

impl GroqTranscriber {
    /// Create a new Groq transcriber.
    ///
    /// Falls back to `GROQ_API_KEY` env var if `api_key` is empty.
    pub fn new(api_key: &str) -> Self {
        let key = if api_key.is_empty() {
            std::env::var("GROQ_API_KEY").unwrap_or_default()
        } else {
            api_key.to_string()
        };

        Self {
            api_key: key,
            api_url: "https://api.groq.com/openai/v1/audio/transcriptions".into(),
            model: "whisper-large-v3".into(),
            client: reqwest::Client::new(),
        }
    }

    /// Create with a custom API URL (for other OpenAI-compatible endpoints).
    pub fn with_url(api_key: &str, api_url: &str) -> Self {
        let mut t = Self::new(api_key);
        t.api_url = api_url.to_string();
        t
    }

    /// Check if the transcriber is configured (has an API key).
    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }
}

#[async_trait]
impl TranscriptionProvider for GroqTranscriber {
    async fn transcribe(&self, file_path: &Path) -> anyhow::Result<String> {
        if !self.is_configured() {
            warn!("groq transcription: no API key configured, skipping");
            return Ok(String::new());
        }

        if !file_path.exists() {
            warn!(path = %file_path.display(), "transcription: file not found");
            return Ok(String::new());
        }

        let file_name = file_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        debug!(
            path = %file_path.display(),
            model = %self.model,
            "transcribing audio via Groq"
        );

        let file_bytes = tokio::fs::read(file_path).await?;

        let file_part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str("application/octet-stream")?;

        let form = reqwest::multipart::Form::new()
            .part("file", file_part)
            .text("model", self.model.clone());

        let response = self
            .client
            .post(&self.api_url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .timeout(Duration::from_secs(60))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!(
                status = %status,
                body = %body,
                "groq transcription API error"
            );
            return Err(anyhow::anyhow!(
                "transcription API returned {}: {}",
                status,
                body
            ));
        }

        let json: serde_json::Value = response.json().await?;
        let text = json["text"].as_str().unwrap_or_default().to_string();

        debug!(
            chars = text.len(),
            "transcription complete"
        );

        Ok(text)
    }

    fn display_name(&self) -> &str {
        "Groq Whisper"
    }
}

// ─────────────────────────────────────────────
// Helper
// ─────────────────────────────────────────────

/// Check if a file path looks like an audio file.
pub fn is_audio_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".ogg")
        || lower.ends_with(".oga")
        || lower.ends_with(".opus")
        || lower.ends_with(".mp3")
        || lower.ends_with(".m4a")
        || lower.ends_with(".wav")
        || lower.ends_with(".flac")
        || lower.ends_with(".aac")
        || lower.ends_with(".wma")
        || lower.ends_with(".webm")
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_audio_file() {
        assert!(is_audio_file("voice.ogg"));
        assert!(is_audio_file("song.MP3"));
        assert!(is_audio_file("/tmp/media/audio.m4a"));
        assert!(is_audio_file("recording.wav"));
        assert!(is_audio_file("file.flac"));
        assert!(is_audio_file("file.opus"));
        assert!(!is_audio_file("photo.jpg"));
        assert!(!is_audio_file("document.pdf"));
        assert!(!is_audio_file("video.mp4"));
    }

    #[test]
    fn test_groq_transcriber_not_configured() {
        let t = GroqTranscriber::new("");
        // Without GROQ_API_KEY env var, should not be configured
        // (this test might see the env var, so just check it doesn't panic)
        let _ = t.is_configured();
    }

    #[test]
    fn test_groq_transcriber_configured() {
        let t = GroqTranscriber::new("gsk_test_key_123");
        assert!(t.is_configured());
        assert_eq!(t.display_name(), "Groq Whisper");
    }

    #[test]
    fn test_groq_transcriber_with_url() {
        let t = GroqTranscriber::with_url("key", "https://custom.api/v1/audio/transcriptions");
        assert_eq!(t.api_url, "https://custom.api/v1/audio/transcriptions");
    }

    #[tokio::test]
    async fn test_transcribe_file_not_found() {
        let t = GroqTranscriber::new("test-key");
        let result = t.transcribe(Path::new("/nonexistent/audio.ogg")).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
