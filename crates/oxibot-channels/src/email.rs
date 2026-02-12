//! Email channel — IMAP polling + SMTP sending.
//!
//! Port of nanobot's `channels/email.py`.
//!
//! Uses a minimal async IMAP client (raw TCP + TLS) for receiving
//! emails and `lettre` for SMTP sending. Polls IMAP for UNSEEN
//! messages at a configurable interval.
//!
//! Features:
//! - IMAP/IMAPS polling for unread emails
//! - SMTP/SMTPS sending via lettre
//! - Allow-list by sender email address
//! - Thread tracking via subject prefix (Re:)
//! - HTML-to-text conversion for inbound emails
//! - Body truncation for long emails
//! - UID-based deduplication
//! - New IMAP connection per poll cycle (matching nanobot)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Mutex, Notify, RwLock};
use tracing::{debug, error, info, warn};

use oxibot_core::bus::queue::MessageBus;
use oxibot_core::bus::types::{InboundMessage, OutboundMessage};
use oxibot_core::config::schema::EmailConfig;

use crate::base::Channel;

// ─────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────

/// Minimum poll interval in seconds.
const MIN_POLL_INTERVAL_SECS: u64 = 5;

/// Default max body characters.
const DEFAULT_MAX_BODY_CHARS: usize = 12000;

/// Default subject prefix for replies.
const DEFAULT_SUBJECT_PREFIX: &str = "Re: ";

/// Maximum tracked UIDs before clearing set.
const MAX_PROCESSED_UIDS: usize = 100_000;

/// Default IMAP port (SSL).
const DEFAULT_IMAP_PORT: u16 = 993;

/// Default SMTP port (STARTTLS).
const DEFAULT_SMTP_PORT: u16 = 587;

// ─────────────────────────────────────────────
// Parsed email struct
// ─────────────────────────────────────────────

/// Extracted data from a parsed email.
#[derive(Debug, Clone)]
struct ParsedEmail {
    /// Sender email address (lowercase).
    sender: String,
    /// Email subject.
    subject: String,
    /// Date header value.
    date: String,
    /// Message-ID header.
    message_id: String,
    /// Text body (plain text; HTML converted).
    body: String,
}

// ─────────────────────────────────────────────
// Minimal async IMAP client
// ─────────────────────────────────────────────

/// Async read+write stream marker.
trait ImapStream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> ImapStream for T {}

/// A minimal async IMAP client supporting only the commands needed
/// to poll for new emails: LOGIN, SELECT, SEARCH, FETCH, STORE, LOGOUT.
struct ImapClient {
    reader: tokio::io::BufReader<tokio::io::ReadHalf<Box<dyn ImapStream>>>,
    writer: tokio::io::WriteHalf<Box<dyn ImapStream>>,
    tag_counter: u32,
}

impl ImapClient {
    /// Connect to an IMAP server (plain or IMAPS/TLS).
    async fn connect(host: &str, port: u16, use_ssl: bool) -> anyhow::Result<Self> {
        use tokio::io::BufReader;
        use tokio::net::TcpStream;

        let tcp = TcpStream::connect((host, port)).await?;

        let stream: Box<dyn ImapStream> = if use_ssl {
            let mut root_store = rustls::RootCertStore::empty();
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

            let config = rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth();

            let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
            let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
                .map_err(|e| anyhow::anyhow!("invalid server name '{}': {}", host, e))?;
            let tls = connector.connect(server_name, tcp).await?;
            Box::new(tls)
        } else {
            Box::new(tcp)
        };

        let (read, write) = tokio::io::split(stream);
        let mut client = Self {
            reader: BufReader::new(read),
            writer: write,
            tag_counter: 0,
        };

        // Read server greeting (e.g. "* OK IMAP server ready")
        let greeting = client.read_line().await?;
        if !greeting.starts_with("* OK") && !greeting.starts_with("* ok") {
            anyhow::bail!("unexpected IMAP greeting: {}", greeting);
        }
        debug!(greeting = %greeting, "IMAP connected");

        Ok(client)
    }

    /// Read a single CRLF-terminated line.
    async fn read_line(&mut self) -> anyhow::Result<String> {
        use tokio::io::AsyncBufReadExt;
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!("IMAP connection closed unexpectedly");
        }
        Ok(line
            .trim_end_matches("\r\n")
            .trim_end_matches('\n')
            .to_string())
    }

    /// Read exactly `n` bytes.
    async fn read_exact(&mut self, n: usize) -> anyhow::Result<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; n];
        self.reader.read_exact(&mut buf).await?;
        Ok(buf)
    }

    /// Send a tagged IMAP command. Returns the tag.
    async fn send_command(&mut self, cmd: &str) -> anyhow::Result<String> {
        use tokio::io::AsyncWriteExt;
        self.tag_counter += 1;
        let tag = format!("A{:04}", self.tag_counter);
        let line = format!("{} {}\r\n", tag, cmd);
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(tag)
    }

    /// Read responses until the tagged completion line.
    /// Returns (untagged_lines, tagged_status_line).
    async fn read_response(&mut self, tag: &str) -> anyhow::Result<(Vec<String>, String)> {
        let mut untagged = Vec::new();
        loop {
            let line = self.read_line().await?;
            if line.starts_with(tag) {
                return Ok((untagged, line));
            }
            untagged.push(line);
        }
    }

    /// LOGIN
    async fn login(&mut self, user: &str, pass: &str) -> anyhow::Result<()> {
        let cmd = format!(
            "LOGIN \"{}\" \"{}\"",
            user.replace('\\', "\\\\").replace('"', "\\\""),
            pass.replace('\\', "\\\\").replace('"', "\\\""),
        );
        let tag = self.send_command(&cmd).await?;
        let (_, status) = self.read_response(&tag).await?;
        if !status.to_uppercase().contains("OK") {
            anyhow::bail!("IMAP LOGIN failed: {}", status);
        }
        Ok(())
    }

    /// SELECT mailbox
    async fn select(&mut self, mailbox: &str) -> anyhow::Result<()> {
        let cmd = format!("SELECT \"{}\"", mailbox);
        let tag = self.send_command(&cmd).await?;
        let (_, status) = self.read_response(&tag).await?;
        if !status.to_uppercase().contains("OK") {
            anyhow::bail!("IMAP SELECT failed: {}", status);
        }
        Ok(())
    }

    /// SEARCH UNSEEN — returns message sequence numbers.
    async fn search_unseen(&mut self) -> anyhow::Result<Vec<u32>> {
        let tag = self.send_command("SEARCH UNSEEN").await?;
        let (lines, status) = self.read_response(&tag).await?;
        if !status.to_uppercase().contains("OK") {
            anyhow::bail!("IMAP SEARCH failed: {}", status);
        }

        let mut seqnums = Vec::new();
        for line in &lines {
            let upper = line.to_uppercase();
            if upper.starts_with("* SEARCH") {
                let nums: Vec<u32> = line
                    .split_whitespace()
                    .skip(2) // skip "* SEARCH"
                    .filter_map(|s| s.parse().ok())
                    .collect();
                seqnums.extend(nums);
            }
        }
        Ok(seqnums)
    }

    /// FETCH a single message by sequence number.
    /// Returns (UID, raw_email_bytes).
    async fn fetch_message(&mut self, seqnum: u32) -> anyhow::Result<(String, Vec<u8>)> {
        let cmd = format!("FETCH {} (UID BODY.PEEK[])", seqnum);
        let tag = self.send_command(&cmd).await?;

        let mut uid = String::new();
        let mut email_data = Vec::new();

        loop {
            let line = self.read_line().await?;

            // Tagged response = done
            if line.starts_with(&tag) {
                if !line.to_uppercase().contains("OK") {
                    anyhow::bail!("IMAP FETCH failed: {}", line);
                }
                break;
            }

            // Untagged FETCH response: * N FETCH (UID nnn BODY[] {size}
            if line.starts_with("* ") && line.to_uppercase().contains("FETCH") {
                // Extract UID
                let upper = line.to_uppercase();
                if let Some(uid_pos) = upper.find("UID ") {
                    let uid_start = uid_pos + 4;
                    let rest = &line[uid_start..];
                    let uid_end = rest
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(rest.len());
                    uid = rest[..uid_end].to_string();
                }

                // Extract literal size {N}
                if let Some(brace_start) = line.rfind('{') {
                    if let Some(brace_end) = line.rfind('}') {
                        if brace_end > brace_start {
                            if let Ok(size) = line[brace_start + 1..brace_end].parse::<usize>() {
                                email_data = self.read_exact(size).await?;
                                // Read closing line(s) after literal data
                                let _closing = self.read_line().await?;
                            }
                        }
                    }
                }
            }
        }

        Ok((uid, email_data))
    }

    /// STORE +FLAGS (\Seen)
    async fn store_seen(&mut self, seqnum: u32) -> anyhow::Result<()> {
        let cmd = format!("STORE {} +FLAGS (\\Seen)", seqnum);
        let tag = self.send_command(&cmd).await?;
        let (_, status) = self.read_response(&tag).await?;
        if !status.to_uppercase().contains("OK") {
            anyhow::bail!("IMAP STORE failed: {}", status);
        }
        Ok(())
    }

    /// LOGOUT
    async fn logout(&mut self) -> anyhow::Result<()> {
        let tag = self.send_command("LOGOUT").await?;
        // Server may send * BYE before the tagged OK
        let _ = self.read_response(&tag).await;
        Ok(())
    }
}

// ─────────────────────────────────────────────
// EmailChannel
// ─────────────────────────────────────────────

/// Email channel — IMAP polling for inbound, SMTP for outbound.
pub struct EmailChannel {
    /// Full config.
    config: EmailConfig,
    /// Message bus.
    bus: Arc<MessageBus>,
    /// Shutdown signal.
    shutdown: Arc<Notify>,
    /// UID deduplication set.
    processed_uids: Arc<Mutex<HashSet<String>>>,
    /// Last inbound subject per sender (for Re: prefix).
    last_subject: Arc<RwLock<HashMap<String, String>>>,
    /// Last inbound Message-ID per sender (for In-Reply-To).
    last_message_id: Arc<RwLock<HashMap<String, String>>>,
}

impl EmailChannel {
    /// Create a new email channel.
    pub fn new(config: EmailConfig, bus: Arc<MessageBus>) -> Self {
        Self {
            config,
            bus,
            shutdown: Arc::new(Notify::new()),
            processed_uids: Arc::new(Mutex::new(HashSet::new())),
            last_subject: Arc::new(RwLock::new(HashMap::new())),
            last_message_id: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // ─────────────────────────────────────────
    // Access control
    // ─────────────────────────────────────────

    /// Check if a sender email is in the allow-list.
    fn is_allowed(&self, sender: &str) -> bool {
        if self.config.allowed_users.is_empty() {
            return true;
        }
        let sender_lower = sender.to_lowercase();
        self.config
            .allowed_users
            .iter()
            .any(|u| u.to_lowercase() == sender_lower)
    }

    /// Effective poll interval (minimum 5 seconds).
    fn poll_interval(&self) -> Duration {
        let secs = (self.config.poll_interval_seconds as u64).max(MIN_POLL_INTERVAL_SECS);
        Duration::from_secs(secs)
    }

    // ─────────────────────────────────────────
    // Email parsing helpers
    // ─────────────────────────────────────────

    /// Extract the email address from a From header value.
    ///
    /// Handles formats like:
    /// - `user@example.com`
    /// - `"User Name" <user@example.com>`
    /// - `User Name <user@example.com>`
    fn extract_sender_email(from_header: &str) -> String {
        // Look for <email> pattern
        if let Some(start) = from_header.rfind('<') {
            if let Some(end) = from_header.rfind('>') {
                if end > start {
                    return from_header[start + 1..end].trim().to_lowercase();
                }
            }
        }
        // Fallback: use the whole thing
        from_header.trim().to_lowercase()
    }

    /// Convert minimal HTML to plain text.
    fn html_to_text(html: &str) -> String {
        let mut text = html.to_string();
        // <br> → newline
        text = regex::Regex::new(r"(?i)<br\s*/?>")
            .unwrap()
            .replace_all(&text, "\n")
            .to_string();
        // </p> → newline
        text = regex::Regex::new(r"(?i)</p>")
            .unwrap()
            .replace_all(&text, "\n")
            .to_string();
        // Strip all remaining tags
        text = regex::Regex::new(r"<[^>]+>")
            .unwrap()
            .replace_all(&text, "")
            .to_string();
        // Unescape common HTML entities
        text = text
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ");
        text.trim().to_string()
    }

    /// Parse a raw RFC2822 email into structured fields.
    fn parse_email(raw: &[u8], max_body_chars: usize) -> Option<ParsedEmail> {
        let parsed = mailparse::parse_mail(raw).ok()?;

        // Extract headers
        let get_header = |name: &str| -> String {
            parsed
                .headers
                .iter()
                .find(|h| h.get_key().to_lowercase() == name.to_lowercase())
                .map(|h| h.get_value())
                .unwrap_or_default()
        };

        let from_raw = get_header("From");
        let sender = Self::extract_sender_email(&from_raw);
        let subject = get_header("Subject");
        let date = get_header("Date");
        let message_id = get_header("Message-ID");

        // Extract body
        let body = Self::extract_body(&parsed, max_body_chars);

        Some(ParsedEmail {
            sender,
            subject,
            date,
            message_id,
            body,
        })
    }

    /// Extract text body from parsed email (prefer text/plain, fallback HTML).
    fn extract_body(mail: &mailparse::ParsedMail, max_chars: usize) -> String {
        if mail.subparts.is_empty() {
            // Single-part message
            let ct = mail.ctype.mimetype.to_lowercase();
            let body = mail.get_body().unwrap_or_default();
            let result = if ct.contains("text/html") {
                Self::html_to_text(&body)
            } else {
                body
            };
            return Self::truncate(&result, max_chars);
        }

        // Multipart: collect text/plain and text/html parts
        let mut plain_parts = Vec::new();
        let mut html_parts = Vec::new();
        Self::collect_text_parts(mail, &mut plain_parts, &mut html_parts);

        let body = if !plain_parts.is_empty() {
            plain_parts.join("\n")
        } else if !html_parts.is_empty() {
            html_parts
                .iter()
                .map(|h| Self::html_to_text(h))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            String::new()
        };

        Self::truncate(&body, max_chars)
    }

    /// Recursively collect text parts from multipart emails.
    fn collect_text_parts(
        mail: &mailparse::ParsedMail,
        plain: &mut Vec<String>,
        html: &mut Vec<String>,
    ) {
        for part in &mail.subparts {
            // Skip attachments
            let disposition = part.get_content_disposition();
            if disposition.disposition == mailparse::DispositionType::Attachment {
                continue;
            }

            if !part.subparts.is_empty() {
                Self::collect_text_parts(part, plain, html);
            } else {
                let ct = part.ctype.mimetype.to_lowercase();
                if let Ok(body) = part.get_body() {
                    if ct.contains("text/plain") {
                        plain.push(body);
                    } else if ct.contains("text/html") {
                        html.push(body);
                    }
                }
            }
        }
    }

    /// Truncate a string to max characters.
    fn truncate(s: &str, max: usize) -> String {
        if s.len() <= max {
            s.to_string()
        } else {
            s[..max].to_string()
        }
    }

    /// Build the subject for a reply.
    fn build_reply_subject(original_subject: &str, prefix: &str) -> String {
        if original_subject.is_empty() {
            return format!("{}(no subject)", prefix);
        }
        if original_subject.to_lowercase().starts_with("re:") {
            return original_subject.to_string();
        }
        format!("{}{}", prefix, original_subject)
    }

    /// Validate that required IMAP config fields are present.
    fn validate_imap_config(&self) -> bool {
        let mut valid = true;
        if self.config.imap_host.is_empty() {
            warn!("email: imap_host not configured");
            valid = false;
        }
        if self.config.imap_username.is_empty() {
            warn!("email: imap_username not configured");
            valid = false;
        }
        if self.config.imap_password.is_empty() {
            warn!("email: imap_password not configured");
            valid = false;
        }
        valid
    }

    // ─────────────────────────────────────────
    // IMAP polling
    // ─────────────────────────────────────────

    /// Poll IMAP once: connect → search unseen → fetch → process → close.
    async fn poll_once(&self) -> anyhow::Result<()> {
        let port = if self.config.imap_port > 0 {
            self.config.imap_port
        } else {
            DEFAULT_IMAP_PORT
        };
        let mailbox = if self.config.imap_mailbox.is_empty() {
            "INBOX"
        } else {
            &self.config.imap_mailbox
        };
        let max_body = if self.config.max_body_chars > 0 {
            self.config.max_body_chars as usize
        } else {
            DEFAULT_MAX_BODY_CHARS
        };

        // Connect
        let mut imap =
            ImapClient::connect(&self.config.imap_host, port, self.config.imap_use_ssl).await?;

        // Login
        imap.login(&self.config.imap_username, &self.config.imap_password)
            .await?;

        // Select mailbox
        imap.select(mailbox).await?;

        // Search unseen
        let seqnums = imap.search_unseen().await?;
        debug!(count = seqnums.len(), "found unseen emails");

        // Fetch each message
        for seqnum in seqnums {
            let (uid, raw) = match imap.fetch_message(seqnum).await {
                Ok(r) => r,
                Err(e) => {
                    warn!(seqnum = seqnum, error = %e, "failed to fetch email");
                    continue;
                }
            };

            // Dedup by UID
            {
                let mut uids = self.processed_uids.lock().await;
                if uids.contains(&uid) {
                    debug!(uid = %uid, "skipping already-processed email");
                    continue;
                }
                if uids.len() >= MAX_PROCESSED_UIDS {
                    uids.clear();
                }
                uids.insert(uid.clone());
            }

            // Parse
            let email = match Self::parse_email(&raw, max_body) {
                Some(e) => e,
                None => {
                    warn!(uid = %uid, "failed to parse email");
                    continue;
                }
            };

            // Allow-list check
            if !self.is_allowed(&email.sender) {
                warn!(sender = %email.sender, "email sender not in allow-list");
                continue;
            }

            // Track subject and message-id for threading
            {
                let mut subjects = self.last_subject.write().await;
                subjects.insert(email.sender.clone(), email.subject.clone());
            }
            if !email.message_id.is_empty() {
                let mut msg_ids = self.last_message_id.write().await;
                msg_ids.insert(email.sender.clone(), email.message_id.clone());
            }

            // Build content string (matching nanobot)
            let content = format!(
                "Email received.\nFrom: {}\nSubject: {}\nDate: {}\n\n{}",
                email.sender, email.subject, email.date, email.body
            );

            // Build metadata
            let mut metadata = HashMap::new();
            metadata.insert("message_id".to_string(), email.message_id);
            metadata.insert("subject".to_string(), email.subject);
            metadata.insert("date".to_string(), email.date);
            metadata.insert("sender_email".to_string(), email.sender.clone());
            metadata.insert("uid".to_string(), uid.clone());

            // Publish inbound
            let inbound = InboundMessage {
                sender_id: email.sender.clone(),
                chat_id: email.sender.clone(), // sender email = chat_id
                channel: "email".to_string(),
                content,
                timestamp: chrono::Utc::now(),
                media: Vec::new(),
                metadata,
            };

            if let Err(e) = self.bus.publish_inbound(inbound).await {
                error!(error = %e, "failed to publish email inbound");
            }

            // Mark as seen
            if self.config.mark_seen {
                if let Err(e) = imap.store_seen(seqnum).await {
                    warn!(seqnum = seqnum, error = %e, "failed to mark email as seen");
                }
            }
        }

        // Logout
        if let Err(e) = imap.logout().await {
            debug!(error = %e, "IMAP logout error (non-fatal)");
        }

        Ok(())
    }

    // ─────────────────────────────────────────
    // SMTP sending
    // ─────────────────────────────────────────

    /// Send an email reply via SMTP using lettre.
    async fn send_email(&self, msg: &OutboundMessage) -> anyhow::Result<()> {
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

        if self.config.smtp_host.is_empty() {
            anyhow::bail!("SMTP host not configured");
        }
        if msg.chat_id.is_empty() {
            anyhow::bail!("no recipient (chat_id is empty)");
        }

        // Determine from address
        let from_addr = if !self.config.from_address.is_empty() {
            &self.config.from_address
        } else if !self.config.smtp_username.is_empty() {
            &self.config.smtp_username
        } else {
            &self.config.imap_username
        };

        if from_addr.is_empty() {
            anyhow::bail!("no from_address configured");
        }

        // Build subject
        let subject = if let Some(s) = msg.metadata.get("subject") {
            s.clone()
        } else {
            let subjects = self.last_subject.read().await;
            let orig = subjects.get(&msg.chat_id).cloned().unwrap_or_default();
            let prefix = if self.config.subject_prefix.is_empty() {
                DEFAULT_SUBJECT_PREFIX
            } else {
                &self.config.subject_prefix
            };
            Self::build_reply_subject(&orig, prefix)
        };

        // Build lettre message
        let email = Message::builder()
            .from(from_addr.parse().map_err(|e| anyhow::anyhow!("invalid from address: {}", e))?)
            .to(msg.chat_id.parse().map_err(|e| anyhow::anyhow!("invalid to address: {}", e))?)
            .subject(&subject)
            .body(msg.content.clone())
            .map_err(|e| anyhow::anyhow!("failed to build email: {}", e))?;

        // Build SMTP transport
        let port = if self.config.smtp_port > 0 {
            self.config.smtp_port
        } else {
            DEFAULT_SMTP_PORT
        };

        let creds = Credentials::new(
            self.config.smtp_username.clone(),
            self.config.smtp_password.clone(),
        );

        let transport = if self.config.smtp_use_ssl {
            // Implicit TLS (SMTPS, port 465)
            AsyncSmtpTransport::<Tokio1Executor>::relay(&self.config.smtp_host)
                .map_err(|e| anyhow::anyhow!("SMTP relay error: {}", e))?
                .port(port)
                .credentials(creds)
                .build()
        } else if self.config.smtp_use_tls {
            // STARTTLS (port 587)
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.config.smtp_host)
                .map_err(|e| anyhow::anyhow!("SMTP STARTTLS error: {}", e))?
                .port(port)
                .credentials(creds)
                .build()
        } else {
            // Plain (no TLS)
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&self.config.smtp_host)
                .port(port)
                .credentials(creds)
                .build()
        };

        transport
            .send(email)
            .await
            .map_err(|e| anyhow::anyhow!("SMTP send error: {}", e))?;

        info!(to = %msg.chat_id, subject = %subject, "email sent");
        Ok(())
    }
}

// ─────────────────────────────────────────────
// Channel trait implementation
// ─────────────────────────────────────────────

#[async_trait]
impl Channel for EmailChannel {
    fn name(&self) -> &str {
        "email"
    }

    async fn start(&self) -> anyhow::Result<()> {
        if !self.validate_imap_config() {
            warn!("email channel not starting: missing IMAP config");
            return Ok(());
        }

        info!(
            imap_host = %self.config.imap_host,
            imap_port = self.config.imap_port,
            mailbox = %self.config.imap_mailbox,
            poll_secs = self.poll_interval().as_secs(),
            "starting email channel"
        );

        let interval = self.poll_interval();

        loop {
            // Poll for new emails
            if let Err(e) = self.poll_once().await {
                warn!(error = %e, "email poll error (will retry)");
            }

            // Wait for interval or shutdown
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = self.shutdown.notified() => {
                    info!("email channel shutting down");
                    return Ok(());
                }
            }
        }
    }

    async fn stop(&self) -> anyhow::Result<()> {
        info!("stopping email channel");
        self.shutdown.notify_waiters();
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> anyhow::Result<()> {
        self.send_email(msg).await
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> EmailConfig {
        EmailConfig {
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            imap_username: "user@example.com".into(),
            imap_password: "password".into(),
            imap_mailbox: "INBOX".into(),
            imap_use_ssl: true,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            smtp_username: "user@example.com".into(),
            smtp_password: "password".into(),
            smtp_use_tls: true,
            smtp_use_ssl: false,
            from_address: "bot@example.com".into(),
            poll_interval_seconds: 30,
            mark_seen: true,
            max_body_chars: 12000,
            subject_prefix: "Re: ".into(),
            allowed_users: Vec::new(),
        }
    }

    fn make_bus() -> Arc<MessageBus> {
        Arc::new(MessageBus::new(10))
    }

    // ── Channel trait ──

    #[test]
    fn test_channel_name() {
        let ch = EmailChannel::new(make_config(), make_bus());
        assert_eq!(ch.name(), "email");
    }

    #[tokio::test]
    async fn test_stop_without_start() {
        let ch = EmailChannel::new(make_config(), make_bus());
        ch.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_start_empty_imap_host() {
        let mut cfg = make_config();
        cfg.imap_host = String::new();
        let ch = EmailChannel::new(cfg, make_bus());
        // Should return Ok without starting the polling loop
        ch.start().await.unwrap();
    }

    // ── Access control ──

    #[test]
    fn test_allowed_empty_list() {
        let ch = EmailChannel::new(make_config(), make_bus());
        assert!(ch.is_allowed("anyone@example.com"));
    }

    #[test]
    fn test_allowed_in_list() {
        let mut cfg = make_config();
        cfg.allowed_users = vec!["alice@example.com".into()];
        let ch = EmailChannel::new(cfg, make_bus());
        assert!(ch.is_allowed("alice@example.com"));
    }

    #[test]
    fn test_allowed_case_insensitive() {
        let mut cfg = make_config();
        cfg.allowed_users = vec!["Alice@Example.COM".into()];
        let ch = EmailChannel::new(cfg, make_bus());
        assert!(ch.is_allowed("alice@example.com"));
    }

    #[test]
    fn test_denied_not_in_list() {
        let mut cfg = make_config();
        cfg.allowed_users = vec!["alice@example.com".into()];
        let ch = EmailChannel::new(cfg, make_bus());
        assert!(!ch.is_allowed("bob@example.com"));
    }

    // ── Poll interval ──

    #[test]
    fn test_poll_interval_default() {
        let ch = EmailChannel::new(make_config(), make_bus());
        assert_eq!(ch.poll_interval(), Duration::from_secs(30));
    }

    #[test]
    fn test_poll_interval_minimum() {
        let mut cfg = make_config();
        cfg.poll_interval_seconds = 2;
        let ch = EmailChannel::new(cfg, make_bus());
        assert_eq!(ch.poll_interval(), Duration::from_secs(MIN_POLL_INTERVAL_SECS));
    }

    // ── Sender extraction ──

    #[test]
    fn test_extract_sender_plain() {
        assert_eq!(
            EmailChannel::extract_sender_email("user@example.com"),
            "user@example.com"
        );
    }

    #[test]
    fn test_extract_sender_with_name() {
        assert_eq!(
            EmailChannel::extract_sender_email("\"John Doe\" <john@example.com>"),
            "john@example.com"
        );
    }

    #[test]
    fn test_extract_sender_angle_brackets() {
        assert_eq!(
            EmailChannel::extract_sender_email("User <USER@Example.COM>"),
            "user@example.com"
        );
    }

    // ── HTML to text ──

    #[test]
    fn test_html_to_text_br() {
        assert_eq!(EmailChannel::html_to_text("Hello<br>World"), "Hello\nWorld");
    }

    #[test]
    fn test_html_to_text_br_self_closing() {
        assert_eq!(EmailChannel::html_to_text("Hello<br/>World"), "Hello\nWorld");
    }

    #[test]
    fn test_html_to_text_paragraph() {
        assert_eq!(
            EmailChannel::html_to_text("<p>Hello</p><p>World</p>"),
            "Hello\nWorld"
        );
    }

    #[test]
    fn test_html_to_text_entities() {
        assert_eq!(
            EmailChannel::html_to_text("&amp; &lt; &gt; &quot; &#39;"),
            "& < > \" '"
        );
    }

    #[test]
    fn test_html_to_text_tags_stripped() {
        assert_eq!(
            EmailChannel::html_to_text("<h1>Title</h1><div>Content</div>"),
            "TitleContent"
        );
    }

    // ── Subject handling ──

    #[test]
    fn test_reply_subject_normal() {
        assert_eq!(
            EmailChannel::build_reply_subject("Hello", "Re: "),
            "Re: Hello"
        );
    }

    #[test]
    fn test_reply_subject_already_re() {
        assert_eq!(
            EmailChannel::build_reply_subject("Re: Hello", "Re: "),
            "Re: Hello"
        );
    }

    #[test]
    fn test_reply_subject_empty() {
        assert_eq!(
            EmailChannel::build_reply_subject("", "Re: "),
            "Re: (no subject)"
        );
    }

    #[test]
    fn test_reply_subject_case_insensitive() {
        assert_eq!(
            EmailChannel::build_reply_subject("RE: Hello", "Re: "),
            "RE: Hello"
        );
    }

    // ── Truncation ──

    #[test]
    fn test_truncate_short() {
        assert_eq!(EmailChannel::truncate("hi", 100), "hi");
    }

    #[test]
    fn test_truncate_exact() {
        assert_eq!(EmailChannel::truncate("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_long() {
        assert_eq!(EmailChannel::truncate("hello world", 5), "hello");
    }

    // ── Email parsing ──

    #[test]
    fn test_parse_simple_email() {
        let raw = b"From: sender@example.com\r\n\
            Subject: Test Email\r\n\
            Date: Mon, 1 Jan 2024 00:00:00 +0000\r\n\
            Message-ID: <abc123@example.com>\r\n\
            Content-Type: text/plain; charset=utf-8\r\n\
            \r\n\
            Hello, this is a test email.\r\n";

        let parsed = EmailChannel::parse_email(raw, 12000).unwrap();
        assert_eq!(parsed.sender, "sender@example.com");
        assert_eq!(parsed.subject, "Test Email");
        assert_eq!(parsed.message_id, "<abc123@example.com>");
        assert!(parsed.body.contains("Hello, this is a test email."));
    }

    #[test]
    fn test_parse_html_email() {
        let raw = b"From: sender@example.com\r\n\
            Subject: HTML Test\r\n\
            Content-Type: text/html; charset=utf-8\r\n\
            \r\n\
            <p>Hello</p><p>World</p>\r\n";

        let parsed = EmailChannel::parse_email(raw, 12000).unwrap();
        assert!(parsed.body.contains("Hello"));
        assert!(parsed.body.contains("World"));
        // Should NOT contain HTML tags
        assert!(!parsed.body.contains("<p>"));
    }

    #[test]
    fn test_parse_email_with_name() {
        let raw = b"From: \"Alice Smith\" <alice@example.com>\r\n\
            Subject: Named\r\n\
            Content-Type: text/plain\r\n\
            \r\n\
            Body\r\n";

        let parsed = EmailChannel::parse_email(raw, 12000).unwrap();
        assert_eq!(parsed.sender, "alice@example.com");
    }

    #[test]
    fn test_parse_email_truncates_body() {
        let raw = format!(
            "From: user@example.com\r\n\
             Subject: Long\r\n\
             Content-Type: text/plain\r\n\
             \r\n\
             {}\r\n",
            "x".repeat(20000)
        );

        let parsed = EmailChannel::parse_email(raw.as_bytes(), 100).unwrap();
        assert_eq!(parsed.body.len(), 100);
    }

    // ── Config validation ──

    #[test]
    fn test_validate_config_complete() {
        let ch = EmailChannel::new(make_config(), make_bus());
        assert!(ch.validate_imap_config());
    }

    #[test]
    fn test_validate_config_missing_host() {
        let mut cfg = make_config();
        cfg.imap_host = String::new();
        let ch = EmailChannel::new(cfg, make_bus());
        assert!(!ch.validate_imap_config());
    }

    #[test]
    fn test_validate_config_missing_username() {
        let mut cfg = make_config();
        cfg.imap_username = String::new();
        let ch = EmailChannel::new(cfg, make_bus());
        assert!(!ch.validate_imap_config());
    }

    #[test]
    fn test_validate_config_missing_password() {
        let mut cfg = make_config();
        cfg.imap_password = String::new();
        let ch = EmailChannel::new(cfg, make_bus());
        assert!(!ch.validate_imap_config());
    }

    // ── Dedup tracking ──

    #[tokio::test]
    async fn test_uid_dedup() {
        let ch = EmailChannel::new(make_config(), make_bus());
        {
            let mut uids = ch.processed_uids.lock().await;
            uids.insert("uid1".to_string());
        }
        let uids = ch.processed_uids.lock().await;
        assert!(uids.contains("uid1"));
        assert!(!uids.contains("uid2"));
    }

    #[tokio::test]
    async fn test_uid_dedup_clear_on_overflow() {
        let ch = EmailChannel::new(make_config(), make_bus());
        {
            let mut uids = ch.processed_uids.lock().await;
            for i in 0..MAX_PROCESSED_UIDS {
                uids.insert(format!("uid{}", i));
            }
            assert_eq!(uids.len(), MAX_PROCESSED_UIDS);
            // Simulating what poll_once does when limit reached
            uids.clear();
        }
        let uids = ch.processed_uids.lock().await;
        assert!(uids.is_empty());
    }

    // ── Subject tracking ──

    #[tokio::test]
    async fn test_subject_tracking() {
        let ch = EmailChannel::new(make_config(), make_bus());
        {
            let mut subjects = ch.last_subject.write().await;
            subjects.insert("user@example.com".into(), "Hello".into());
        }
        let subjects = ch.last_subject.read().await;
        assert_eq!(
            subjects.get("user@example.com").unwrap(),
            "Hello"
        );
    }
}
