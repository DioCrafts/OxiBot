<div align="center">
  <h1>🤖 OxiBot: Ultra-Lightweight Personal AI Assistant in Rust</h1>
  <p>
    <img src="https://img.shields.io/badge/rust-≥1.84-orange?logo=rust" alt="Rust">
    <img src="https://img.shields.io/badge/license-MIT-green" alt="License">
    <img src="https://img.shields.io/badge/tests-273%2B%20passed-brightgreen" alt="Tests">
    <img src="https://img.shields.io/badge/RAM-<%208MB-blue" alt="RAM">
    <img src="https://img.shields.io/badge/binary-static%20single%20file-blue" alt="Binary">
  </p>
  <p><em>A Rust reimplementation of <a href="https://github.com/HKUDS/nanobot">nanobot</a> — delivering the same features with zero runtime dependencies, ultra-low memory, and sub-second startup.</em></p>
</div>

---

🦀 **OxiBot** is an **ultra-lightweight** personal AI assistant built entirely in Rust.

⚡️ Ships as a **single static binary** (~18K lines of Rust) with no Python, no pip, no runtime dependencies.

🎯 **Feature-complete** port of nanobot: same config format, same channels, same skills, same CLI.

| Metric | OxiBot | nanobot |
|--------|--------|---------|
| Language | Rust | Python |
| Binary | Single static file | pip install + 50+ deps |
| RAM | < 8 MB | ~50-100 MB |
| Startup | < 1 s | 2-4 s |
| LOC (core) | ~18,372 | ~3,510 |
| Cross-compile | x86_64, ARM64, RISC-V | Python-only |

## 📦 Install

### From source (recommended)

```bash
git clone https://github.com/DioCrafts/OxiBot.git
cd OxiBot
cargo build --release
```

Binary output: `target/release/oxibot`

### With specific channel features

```bash
# Only Telegram
cargo build --release --features "telegram"

# Multiple channels
cargo build --release --features "telegram,discord,slack"

# All channels (what the Dockerfile uses)
cargo build --release --features "telegram,discord,whatsapp,slack,email"
```

### Feature Flags

| Feature | Description |
|---------|-------------|
| `telegram` | Telegram bot via teloxide |
| `discord` | Discord bot via WebSocket gateway |
| `whatsapp` | WhatsApp via Node.js bridge (Baileys) |
| `slack` | Slack bot via Socket Mode |
| `email` | Email via IMAP + SMTP |

## 🚀 Quick Start

> [!TIP]
> Set your API key in `~/.oxibot/config.json`.
> Get API keys: [OpenRouter](https://openrouter.ai/keys) (recommended) · [Anthropic](https://console.anthropic.com) · [Brave Search](https://brave.com/search/api/) (optional)

**1. Initialize**

```bash
oxibot onboard
```

**2. Configure** (`~/.oxibot/config.json`)

```json
{
  "providers": {
    "openrouter": {
      "apiKey": "sk-or-v1-xxx"
    }
  },
  "agents": {
    "defaults": {
      "model": "anthropic/claude-sonnet-4-20250514"
    }
  }
}
```

**3. Chat**

```bash
oxibot agent -m "What is 2+2?"
```

That's it! Working AI assistant in 2 minutes. 🎉

## 🖥️ Local Models (vLLM)

Run OxiBot with your own local LLMs using vLLM or any OpenAI-compatible server.

```json
{
  "providers": {
    "vllm": {
      "apiKey": "dummy",
      "apiBase": "http://localhost:8000/v1"
    }
  },
  "agents": {
    "defaults": {
      "model": "meta-llama/Llama-3.1-8B-Instruct"
    }
  }
}
```

```bash
oxibot agent -m "Hello from my local LLM!"
```

> [!TIP]
> The `apiKey` can be any non-empty string for local servers that don't require authentication.

## 🏗️ Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        oxibot (single binary)                       │
│                                                                     │
│  ┌──────────┐  ┌───────────────┐  ┌────────────┐  ┌─────────────┐   │
│  │ oxibot-  │  │   oxibot-     │  │  oxibot-   │  │   oxibot-   │   │
│  │  cli     │──│   agent       │──│  providers │  │   cron      │   │
│  │          │  │               │  │            │  │             │   │
│  │ commands │  │ loop, tools,  │  │ 12 LLM     │  │ scheduler,  │   │
│  │ gateway  │  │ memory, ctx   │  │ backends   │  │ jobs, store │   │
│  │ repl     │  │ skills, sub   │  │ + whisper  │  │             │   │
│  └──────────┘  └───────────────┘  └────────────┘  └─────────────┘   │
│       │                │                                    │       │
│  ┌────▼────────────────▼────────────────────────────────────▼────┐  │
│  │                     oxibot-core                               │  │
│  │   config · bus · session · heartbeat · types · utils          │  │
│  └──────────────────────────┬────────────────────────────────────┘  │
│                             │                                       │
│  ┌──────────────────────────▼────────────────────────────────────┐  │
│  │                    oxibot-channels                            │  │
│  │   telegram · discord · whatsapp · slack · email               │  │
│  └───────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
         │ (WhatsApp only)
         │ WebSocket ws://localhost:3001
┌────────▼─────────────────────────┐
│  Node.js WhatsApp Bridge         │
│  (Baileys v7, TypeScript)        │
└──────────────────────────────────┘
```

## 💬 Chat Apps

Talk to OxiBot through Telegram, Discord, WhatsApp, Slack, or Email — anytime, anywhere.

| Channel | Setup | Requires |
|---------|-------|----------|
| **Telegram** | Easy | Bot token |
| **Discord** | Easy | Bot token + intents |
| **WhatsApp** | Medium | Node.js + QR scan |
| **Slack** | Medium | Bot + App tokens |
| **Email** | Medium | IMAP/SMTP credentials |

<details>
<summary><b>Telegram</b> (Recommended)</summary>

**1. Create a bot** — Open Telegram → `@BotFather` → `/newbot` → copy the token

**2. Configure**

```json
{
  "channels": {
    "telegram": {
      "token": "YOUR_BOT_TOKEN",
      "allowedUsers": ["YOUR_USER_ID"]
    }
  }
}
```

**3. Build & Run**

```bash
cargo build --release --features telegram
oxibot gateway
```

</details>

<details>
<summary><b>Discord</b></summary>

**1. Create a bot**
- [Discord Developer Portal](https://discord.com/developers/applications) → Create Application → Bot → Add Bot
- Enable **MESSAGE CONTENT INTENT**
- Copy the bot token

**2. Invite the bot**
- OAuth2 → URL Generator → Scopes: `bot` → Permissions: `Send Messages`, `Read Message History`
- Open the generated URL and add to your server

**3. Configure**

```json
{
  "channels": {
    "discord": {
      "token": "YOUR_BOT_TOKEN",
      "allowedUsers": ["YOUR_USER_ID"]
    }
  }
}
```

**4. Build & Run**

```bash
cargo build --release --features discord
oxibot gateway
```

</details>

<details>
<summary><b>WhatsApp</b></summary>

Requires **Node.js ≥20** for the Baileys bridge.

**1. Build the bridge**

```bash
cd bridge && npm install && npm run build && cd ..
```

**2. Link device**

```bash
oxibot channels login
# Scan QR with WhatsApp → Settings → Linked Devices
```

**3. Configure**

```json
{
  "channels": {
    "whatsapp": {
      "bridgeUrl": "ws://localhost:3001",
      "allowedUsers": ["+1234567890"]
    }
  }
}
```

**4. Run** (two terminals)

```bash
# Terminal 1: Start the bridge
cd bridge && npm start

# Terminal 2: Start the bot
cargo build --release --features whatsapp
oxibot gateway
```

</details>

<details>
<summary><b>Slack</b></summary>

Uses **Socket Mode** — no public URL required.

**1. Create a Slack app**
- [Slack API](https://api.slack.com/apps) → Create New App → "From scratch"
- **Socket Mode**: Toggle ON → Generate App-Level Token (`xapp-...`)
- **OAuth & Permissions**: Add scopes: `chat:write`, `reactions:write`, `app_mentions:read`
- **Event Subscriptions**: Toggle ON → Subscribe: `message.im`, `message.channels`, `app_mention`
- **App Home**: Enable Messages Tab → Allow messages
- **Install to Workspace** → Copy Bot Token (`xoxb-...`)

**2. Configure**

```json
{
  "channels": {
    "slack": {
      "botToken": "xoxb-...",
      "appToken": "xapp-...",
      "groupPolicy": "mention"
    }
  }
}
```

**3. Build & Run**

```bash
cargo build --release --features slack
oxibot gateway
```

> [!TIP]
> `groupPolicy`: `"mention"` (respond to @mentions), `"open"` (all messages), or `"allowlist"`.

</details>

<details>
<summary><b>Email</b></summary>

Polls **IMAP** for incoming mail, replies via **SMTP**.

**1. Get credentials** (Gmail example: enable 2FA → create [App Password](https://myaccount.google.com/apppasswords))

**2. Configure**

```json
{
  "channels": {
    "email": {
      "imapHost": "imap.gmail.com",
      "imapPort": 993,
      "imapUsername": "my-oxibot@gmail.com",
      "imapPassword": "your-app-password",
      "smtpHost": "smtp.gmail.com",
      "smtpPort": 587,
      "smtpUsername": "my-oxibot@gmail.com",
      "smtpPassword": "your-app-password",
      "fromAddress": "my-oxibot@gmail.com",
      "allowedUsers": ["your-real-email@gmail.com"]
    }
  }
}
```

**3. Build & Run**

```bash
cargo build --release --features email
oxibot gateway
```

</details>

## ⚙️ Configuration

Config file: `~/.oxibot/config.json`

### Providers

| Provider | Purpose | Get API Key |
|----------|---------|-------------|
| `openrouter` | LLM (recommended, access to all models) | [openrouter.ai](https://openrouter.ai) |
| `anthropic` | LLM (Claude direct) | [console.anthropic.com](https://console.anthropic.com) |
| `openai` | LLM (GPT direct) | [platform.openai.com](https://platform.openai.com) |
| `deepseek` | LLM (DeepSeek direct) | [platform.deepseek.com](https://platform.deepseek.com) |
| `groq` | LLM + **Voice transcription** (Whisper) | [console.groq.com](https://console.groq.com) |
| `gemini` | LLM (Gemini direct) | [aistudio.google.com](https://aistudio.google.com) |
| `minimax` | LLM (MiniMax direct) | [platform.minimax.io](https://platform.minimax.io) |
| `aihubmix` | LLM (API gateway) | [aihubmix.com](https://aihubmix.com) |
| `dashscope` | LLM (Qwen) | [dashscope.console.aliyun.com](https://dashscope.console.aliyun.com) |
| `moonshot` | LLM (Moonshot/Kimi) | [platform.moonshot.cn](https://platform.moonshot.cn) |
| `zhipu` | LLM (Zhipu GLM) | [open.bigmodel.cn](https://open.bigmodel.cn) |
| `vllm` | LLM (local, any OpenAI-compatible server) | — |

> [!TIP]
> **Groq** provides free voice transcription via Whisper. If configured, Telegram voice messages will be automatically transcribed.

### Environment Variables

All env vars use `OXIBOT_` prefix with `__` as section delimiter:

```bash
export OXIBOT_PROVIDERS__ANTHROPIC__API_KEY=sk-ant-xxx
export OXIBOT_AGENTS__DEFAULTS__MODEL=anthropic/claude-sonnet-4-20250514
export OXIBOT_GATEWAY__PORT=9090
```

Config precedence: **Defaults** → **config.json** → **Environment variables** (env overrides all).

### Security

> For production, set `"restrictToWorkspace": true` to sandbox the agent.

| Option | Default | Description |
|--------|---------|-------------|
| `tools.restrictToWorkspace` | `false` | Restricts all agent tools to workspace directory |
| `channels.*.allowedUsers` | `[]` (allow all) | Whitelist of user IDs. Empty = allow everyone |

See [SECURITY.md](SECURITY.md) for comprehensive security guidance.

## 📖 CLI Reference

| Command | Description |
|---------|-------------|
| `oxibot onboard` | Initialize config & workspace |
| `oxibot agent -m "..."` | Chat (single message) |
| `oxibot agent` | Interactive REPL |
| `oxibot agent --no-markdown` | Plain-text replies |
| `oxibot agent --logs` | Show debug logs |
| `oxibot gateway` | Start all channels + cron + heartbeat |
| `oxibot status` | Show config & provider status |
| `oxibot channels status` | Show channel status |
| `oxibot channels login` | Link WhatsApp (scan QR) |
| `oxibot cron list` | List scheduled jobs |
| `oxibot cron add` | Add a scheduled job |
| `oxibot cron remove <id>` | Remove a job |
| `oxibot cron enable <id>` | Enable/disable a job |
| `oxibot cron run <id>` | Manually trigger a job |

Interactive mode exits: `exit`, `quit`, `/exit`, `/quit`, `:q`, Ctrl-C, Ctrl-D.

<details>
<summary><b>Scheduled Tasks (Cron)</b></summary>

```bash
# Cron expression
oxibot cron add --name "morning" --message "Daily summary" --cron "0 9 * * *"

# Interval (seconds)
oxibot cron add --name "check" --message "Status update" --every 3600

# One-time at specific time
oxibot cron add --name "remind" --message "Call dentist" --at "2026-03-01T09:00:00"

# With channel delivery
oxibot cron add --name "alert" --message "Health check" --every 300 \
  --deliver --channel telegram --to "123456789"

# List / remove
oxibot cron list
oxibot cron remove <job_id>
```

</details>

## 🎯 Skills

Bundled skills in `crates/oxibot-agent/skills/`:

| Skill | Description |
|-------|-------------|
| **skill-creator** | Guides the agent on creating new skills |
| **weather** | Weather via `wttr.in` (no API key needed) |
| **cron** | Schedule reminders and recurring tasks |
| **tmux** | Remote-control tmux sessions |
| **github** | Interact with GitHub via `gh` CLI |
| **summarize** | Summarize URLs and articles |

Custom skills can be added to `~/.oxibot/workspace/skills/`.

## 🐳 Docker

```bash
# Build the image
docker build -t oxibot .

# Initialize (first time)
docker run -v ~/.oxibot:/home/oxibot/.oxibot --rm oxibot onboard

# Edit config to add API keys
vim ~/.oxibot/config.json

# Run gateway
docker run -d \
  -v ~/.oxibot:/home/oxibot/.oxibot \
  -p 18790:18790 \
  oxibot gateway

# Single command
docker run -v ~/.oxibot:/home/oxibot/.oxibot --rm oxibot agent -m "Hello!"
docker run -v ~/.oxibot:/home/oxibot/.oxibot --rm oxibot status
```

> [!TIP]
> The `-v ~/.oxibot:/home/oxibot/.oxibot` flag persists config and workspace across container restarts.

## 📁 Project Structure

```
OxiBot/
├── Cargo.toml                  # Workspace root
├── Dockerfile                  # Multi-stage build (Rust + Node.js bridge)
├── bridge/                     # 🌉 Node.js WhatsApp bridge (Baileys)
│   ├── src/
│   │   ├── index.ts            #    Entry point
│   │   ├── server.ts           #    WebSocket server
│   │   └── whatsapp.ts         #    Baileys client
│   └── package.json
└── crates/
    ├── oxibot-core/            # ⚙️  Config, bus, session, heartbeat, utils
    ├── oxibot-agent/           # 🧠  Agent loop, tools, memory, context, skills
    │   └── skills/             # 🎯  Bundled skills (weather, cron, tmux, etc.)
    ├── oxibot-providers/       # 🤖  12 LLM backends + Whisper transcription
    ├── oxibot-channels/        # 📱  Telegram, Discord, WhatsApp, Slack, Email
    ├── oxibot-cron/            # ⏰  Scheduled task engine
    └── oxibot-cli/             # 🖥️  CLI commands, gateway, REPL
```

## 🧪 Testing

```bash
# Run all tests
cargo test --workspace

# With all channel features
cargo test --workspace --features "telegram,discord,whatsapp,slack,email"

# Specific crate
cargo test -p oxibot-core
cargo test -p oxibot-agent
cargo test -p oxibot-providers
cargo test -p oxibot-channels
cargo test -p oxibot-cron
cargo test -p oxibot-cli
```

See [TESTING-GUIDE.md](TESTING-GUIDE.md) for comprehensive testing procedures, sample configs, and Docker instructions.

## 🤝 Contributing

PRs welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

**Roadmap:**

- [x] Core agent loop (tools, memory, sessions)
- [x] 12 LLM providers
- [x] 5 chat channels (Telegram, Discord, WhatsApp, Slack, Email)
- [x] Cron scheduler
- [x] Heartbeat service
- [x] Voice transcription (Groq Whisper)
- [x] WhatsApp bridge (TypeScript + Baileys)
- [ ] Multi-modal support (images, video)
- [ ] Enhanced long-term memory
- [ ] Web UI dashboard
- [ ] Plugin system for custom tools
- [ ] CI/CD with GitHub Actions

## 📜 License

[MIT](LICENSE)

<p align="center">
  <sub>OxiBot is a Rust reimplementation of <a href="https://github.com/HKUDS/nanobot">nanobot</a> for educational and research purposes.</sub>

</p>
