# 🛡️ Security Policy — OxiBot

OxiBot runs autonomous agent loops that can execute shell commands, read/write files, and interact with external APIs.
This guide covers how to deploy it safely.

---

## 📋 Table of Contents

1. [Reporting Vulnerabilities](#reporting-vulnerabilities)
2. [Threat Model](#threat-model)
3. [Workspace Sandbox](#workspace-sandbox)
4. [User Allowlists](#user-allowlists)
5. [API Key Management](#api-key-management)
6. [Docker Hardening](#docker-hardening)
7. [Network Security](#network-security)
8. [Tool Restrictions](#tool-restrictions)
9. [Logging & Auditing](#logging--auditing)
10. [Security Checklist](#security-checklist)

---

## Reporting Vulnerabilities

If you discover a security issue, **please do NOT open a public GitHub issue.**

Instead, email **security@diocrafts.com** with:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

We will respond within **72 hours** and aim to release a patch within **7 days** for critical issues.

---

## Threat Model

OxiBot is designed as a **personal assistant** — typically running on your own machine or a private server.
The primary threats are:

| Threat | Vector | Mitigation |
|--------|--------|------------|
| **Prompt injection** | Malicious user messages trick the agent into unsafe actions | `allowedUsers` whitelist, workspace sandbox |
| **Data exfiltration** | Agent reads sensitive files and leaks them via tool output | `restrictToWorkspace`, filesystem permissions |
| **Command injection** | Agent constructs dangerous shell commands | Workspace sandbox, Docker isolation |
| **API key theft** | Keys stored in plaintext config | File permissions, env vars, Docker secrets |
| **Unauthorized access** | Unknown users message the bot | `allowedUsers` per channel |
| **Dependency supply chain** | Compromised crate or npm package | Cargo.lock pinning, minimal dependencies |

---

## Workspace Sandbox

The **most important** security control. When enabled, the agent's file/directory tools are restricted to `~/.oxibot/workspace/`.

### Enable

```json
{
  "tools": {
    "restrictToWorkspace": true
  }
}
```

Or via environment variable:
```bash
export OXIBOT_TOOLS__RESTRICT_TO_WORKSPACE=true
```

### What it restricts

| Tool | Unrestricted | Sandboxed |
|------|-------------|-----------|
| File read/write | Entire filesystem | `~/.oxibot/workspace/` only |
| Directory listing | Entire filesystem | `~/.oxibot/workspace/` only |
| Shell commands | Full system access | CWD forced to workspace |

> [!IMPORTANT]
> **Always enable `restrictToWorkspace` in production.** The default is `false` to make development easier, but this gives the agent unrestricted filesystem access.

---

## User Allowlists

Each channel supports an `allowedUsers` array. When set, only listed users can interact with the bot.

```json
{
  "channels": {
    "telegram": {
      "token": "...",
      "allowedUsers": ["123456789"]
    },
    "discord": {
      "token": "...",
      "allowedUsers": ["987654321"]
    },
    "whatsapp": {
      "bridgeUrl": "ws://localhost:3001",
      "allowedUsers": ["+1234567890"]
    },
    "slack": {
      "botToken": "xoxb-...",
      "appToken": "xapp-...",
      "allowedUsers": ["U01ABCDEF"]
    },
    "email": {
      "allowedUsers": ["trusted@example.com"]
    }
  }
}
```

> [!WARNING]
> An **empty** `allowedUsers` array (or omitting it) means **all users are allowed**. Always configure allowlists in production.

### Finding your user ID

| Channel | How to find your ID |
|---------|-------------------|
| Telegram | Message `@userinfobot` or `@RawDataBot` |
| Discord | Settings → Advanced → Developer Mode ON → Right-click yourself → "Copy User ID" |
| WhatsApp | Your phone number with country code (e.g., `+34612345678`) |
| Slack | Click your profile → "Copy member ID" |
| Email | Your email address |

---

## API Key Management

### ❌ Don't

```bash
# Hardcoding keys in shell history
export OXIBOT_PROVIDERS__OPENROUTER__API_KEY=sk-or-v1-abc123  # In .bashrc
```

### ✅ Do

**Option 1: Config file with proper permissions**
```bash
chmod 600 ~/.oxibot/config.json
```

**Option 2: Environment variables from a secrets manager**
```bash
# From 1Password
export OXIBOT_PROVIDERS__OPENROUTER__API_KEY=$(op read "op://Private/OxiBot/api-key")
```

**Option 3: Docker secrets**
```yaml
# docker-compose.yml
services:
  oxibot:
    image: oxibot
    secrets:
      - oxibot_api_key
    environment:
      OXIBOT_PROVIDERS__OPENROUTER__API_KEY_FILE: /run/secrets/oxibot_api_key

secrets:
  oxibot_api_key:
    file: ./api_key.txt
```

### Key rotation

Rotate API keys periodically:
1. Generate new key at provider dashboard
2. Update `config.json` or environment variable
3. Restart OxiBot (`oxibot gateway`)
4. Revoke old key at provider dashboard

---

## Docker Hardening

The OxiBot Dockerfile already runs as non-root user `oxibot`. Additional hardening:

```bash
docker run -d \
  --name oxibot \
  --read-only \
  --tmpfs /tmp \
  --security-opt no-new-privileges:true \
  --cap-drop ALL \
  --memory 128m \
  --cpus 0.5 \
  -v ~/.oxibot:/home/oxibot/.oxibot:rw \
  -p 127.0.0.1:18790:18790 \
  oxibot gateway
```

| Flag | Purpose |
|------|---------|
| `--read-only` | Immutable container filesystem |
| `--tmpfs /tmp` | Writable temp directory (in RAM) |
| `--no-new-privileges` | Prevent privilege escalation |
| `--cap-drop ALL` | Remove all Linux capabilities |
| `--memory 128m` | Limit memory usage |
| `--cpus 0.5` | Limit CPU usage |
| `-p 127.0.0.1:18790` | Bind only to localhost |

### Docker Compose (production)

```yaml
version: "3.8"
services:
  oxibot:
    build: .
    restart: unless-stopped
    read_only: true
    security_opt:
      - no-new-privileges:true
    cap_drop:
      - ALL
    mem_limit: 128m
    cpus: 0.5
    tmpfs:
      - /tmp
    volumes:
      - ~/.oxibot:/home/oxibot/.oxibot:rw
    ports:
      - "127.0.0.1:18790:18790"
```

---

## Network Security

### Outbound connections

OxiBot makes outbound HTTPS calls to:
- LLM provider APIs (OpenRouter, Anthropic, OpenAI, etc.)
- Channel APIs (Telegram Bot API, Discord Gateway, etc.)
- Tool URLs (web_get, web_search via Brave)

### Inbound connections

OxiBot does **not** expose any HTTP server by default. All channels use **outbound polling or WebSocket connections**:

| Channel | Connection Type |
|---------|----------------|
| Telegram | Long-polling (outbound) |
| Discord | WebSocket (outbound) |
| WhatsApp | WebSocket to local bridge |
| Slack | Socket Mode (outbound) |
| Email | IMAP polling (outbound) |

The only inbound port is the **heartbeat/health** endpoint on port `18790` (configurable, optional).

### Firewall rules (production)

```bash
# Allow outbound HTTPS only
iptables -A OUTPUT -p tcp --dport 443 -j ACCEPT
iptables -A OUTPUT -p tcp --dport 993 -j ACCEPT  # IMAP (if using email)
iptables -A OUTPUT -p tcp --dport 587 -j ACCEPT  # SMTP (if using email)

# Block all other outbound
iptables -A OUTPUT -j DROP
```

---

## Tool Restrictions

The agent has access to these built-in tools:

| Tool | Risk Level | Notes |
|------|-----------|-------|
| `bash` | 🔴 High | Executes arbitrary shell commands |
| `read_file` | 🟡 Medium | Can read sensitive files |
| `write_file` | 🟡 Medium | Can overwrite files |
| `list_dir` | 🟢 Low | Directory listing |
| `web_get` | 🟡 Medium | HTTP requests to arbitrary URLs |
| `web_search` | 🟢 Low | Search via Brave API |

**Mitigations:**
- Enable `restrictToWorkspace` to sandbox file/dir tools
- Use `allowedUsers` to restrict who can trigger tool use
- Run in Docker with minimal capabilities
- Monitor agent logs for suspicious activity

---

## Logging & Auditing

Enable verbose logging to track agent activity:

```bash
# Full debug logs
RUST_LOG=debug oxibot gateway

# Agent-specific logs
RUST_LOG=oxibot_agent=debug oxibot gateway

# Log to file
RUST_LOG=info oxibot gateway 2>&1 | tee -a /var/log/oxibot.log
```

Key events to monitor:
- `tool_call: bash` — Shell commands executed
- `tool_call: write_file` — Files modified
- `incoming_message` — User messages received
- `auth_rejected` — Unauthorized access attempts

---

## Security Checklist

Use this checklist before deploying OxiBot in production:

### Essential (must do)

- [ ] Set `restrictToWorkspace: true`
- [ ] Configure `allowedUsers` for every channel
- [ ] Set `chmod 600 ~/.oxibot/config.json`
- [ ] Use environment variables or secrets manager for API keys
- [ ] Run as non-root user (Docker image does this by default)

### Recommended

- [ ] Run in Docker with hardening flags
- [ ] Use `--read-only` container filesystem
- [ ] Limit container memory and CPU
- [ ] Bind health port to localhost only
- [ ] Enable verbose logging for auditing
- [ ] Set up log rotation

### Advanced

- [ ] Use Docker secrets for API keys
- [ ] Configure firewall rules (outbound HTTPS only)
- [ ] Run in dedicated VM or namespace
- [ ] Set up alerting on `auth_rejected` events
- [ ] Regular API key rotation schedule
- [ ] Periodic dependency audit (`cargo audit`)

---

<p align="center">
  <sub>Security is a shared responsibility. If you find a vulnerability, please report it responsibly.</sub>
</p>
