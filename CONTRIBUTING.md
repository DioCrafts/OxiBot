# 🤝 Contributing to OxiBot

Thanks for your interest in contributing to OxiBot! This document provides guidelines and
instructions for contributing.

---

## 📋 Table of Contents

1. [Getting Started](#getting-started)
2. [Development Setup](#development-setup)
3. [Code Style](#code-style)
4. [Testing](#testing)
5. [Pull Request Process](#pull-request-process)
6. [Project Structure](#project-structure)
7. [Adding a New Provider](#adding-a-new-provider)
8. [Adding a New Channel](#adding-a-new-channel)
9. [Adding a New Skill](#adding-a-new-skill)
10. [Commit Convention](#commit-convention)

---

## Getting Started

1. **Fork** the repository on GitHub
2. **Clone** your fork locally
3. **Create a branch** for your feature or fix
4. **Make changes** following the guidelines below
5. **Submit a PR** with a clear description

## Development Setup

### Prerequisites

- **Rust** ≥ 1.84 (install via [rustup](https://rustup.rs/))
- **Node.js** ≥ 20 (only for WhatsApp bridge development)
- **cargo-watch** (optional, for live reload): `cargo install cargo-watch`

### Build

```bash
git clone https://github.com/DioCrafts/OxiBot.git
cd OxiBot

# Debug build (faster compilation)
cargo build --workspace

# Release build
cargo build --release --features "telegram,discord,whatsapp,slack,email"

# Watch mode (rebuild on changes)
cargo watch -x "build --workspace"
```

### Run tests

```bash
cargo test --workspace
cargo test --workspace --features "telegram,discord,whatsapp,slack,email"
```

---

## Code Style

We follow standard Rust conventions with a few project-specific guidelines.

### Formatting

```bash
# Format all code (required before PR)
cargo fmt --all

# Check formatting without modifying
cargo fmt --all -- --check
```

### Linting

```bash
# Run clippy (required: zero warnings)
cargo clippy --workspace --all-targets -- -D warnings

# With all features
cargo clippy --workspace --all-targets --features "telegram,discord,whatsapp,slack,email" -- -D warnings
```

### Guidelines

- **Use `thiserror`** for error types, not manual `impl Display`
- **Use `tracing`** for logging, not `println!` or `eprintln!`
- **Async by default** — use `tokio` for async runtime
- **Prefer `&str` over `String`** in function parameters when ownership isn't needed
- **Document public APIs** — all `pub` items should have `///` doc comments
- **No `unwrap()` in library code** — use `?` or proper error handling
- **`unwrap()` is OK in tests** and in CLI entry points with clear error context

### Naming conventions

| Item | Convention | Example |
|------|-----------|---------|
| Crates | `oxibot-*` kebab-case | `oxibot-core` |
| Modules | `snake_case` | `agent_loop` |
| Types/Traits | `PascalCase` | `AgentConfig` |
| Functions | `snake_case` | `process_message` |
| Constants | `SCREAMING_SNAKE_CASE` | `MAX_RETRIES` |
| Feature flags | `lowercase` | `telegram`, `discord` |

---

## Testing

### Writing tests

- Place **unit tests** in the same file, inside `#[cfg(test)] mod tests { ... }`
- Place **integration tests** in `tests/` directory of the crate
- Use `#[tokio::test]` for async tests
- Use `assert!`, `assert_eq!`, `assert_ne!` — not `unwrap()` for assertions

### Test example

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = Config::default();
        assert_eq!(config.gateway.port, 18790);
    }

    #[tokio::test]
    async fn test_provider_sends_message() {
        let provider = MockProvider::new();
        let response = provider.chat("Hello").await.unwrap();
        assert!(!response.is_empty());
    }
}
```

### Test coverage

```bash
# Install cargo-tarpaulin (Linux only)
cargo install cargo-tarpaulin

# Run coverage
cargo tarpaulin --workspace --out Html
```

---

## Pull Request Process

### Before submitting

- [ ] Code compiles: `cargo build --workspace`
- [ ] All tests pass: `cargo test --workspace`
- [ ] Code is formatted: `cargo fmt --all`
- [ ] No clippy warnings: `cargo clippy --workspace -- -D warnings`
- [ ] New code has tests
- [ ] Public APIs are documented

### PR description template

```markdown
## What

Brief description of the change.

## Why

Motivation or issue reference.

## How

Technical approach taken.

## Testing

How this was tested.
```

### Review process

1. Submit PR against `main` branch
2. CI runs (build + test + clippy + fmt check)
3. At least one maintainer review required
4. Squash-merge when approved

---

## Project Structure

```
crates/
├── oxibot-core/       # Config, event bus, session, heartbeat, types
├── oxibot-agent/      # Agent loop, tools, memory, context, skills
├── oxibot-providers/  # LLM provider backends (OpenAI, Anthropic, etc.)
├── oxibot-channels/   # Chat channel implementations
├── oxibot-cron/       # Scheduled task engine
└── oxibot-cli/        # CLI commands and gateway
```

### Dependency flow

```
cli → agent → providers
 │      │
 │      └──→ core
 │             ↑
 ├──→ channels─┘
 │
 └──→ cron───→ core
```

**Rules:**
- `core` depends on nothing internal (only external crates)
- `providers` depends on `core`
- `agent` depends on `core` and `providers`
- `channels` depends on `core`
- `cron` depends on `core`
- `cli` depends on everything

---

## Adding a New Provider

1. Create `crates/oxibot-providers/src/your_provider.rs`
2. Implement the `LlmProvider` trait:

```rust
pub struct YourProvider {
    client: reqwest::Client,
    api_key: String,
    api_base: String,
}

#[async_trait]
impl LlmProvider for YourProvider {
    fn name(&self) -> &str { "your_provider" }
    
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        // Implement API call
    }
}
```

3. Register in `crates/oxibot-providers/src/lib.rs`
4. Add config fields in `crates/oxibot-core/src/config.rs`
5. Add tests
6. Update README.md provider table

---

## Adding a New Channel

1. Create `crates/oxibot-channels/src/your_channel.rs`
2. Implement the `Channel` trait:

```rust
pub struct YourChannel { /* ... */ }

#[async_trait]
impl Channel for YourChannel {
    fn name(&self) -> &str { "your_channel" }
    
    async fn start(&self, bus: EventBus) -> Result<()> {
        // Connect and start receiving messages
    }
}
```

3. Add a feature flag in `crates/oxibot-channels/Cargo.toml`:

```toml
[features]
your_channel = ["dep:your-channel-sdk"]
```

4. Gate the module with `#[cfg(feature = "your_channel")]`
5. Register in CLI gateway startup
6. Add config fields and tests
7. Update README.md channel table

---

## Adding a New Skill

Skills are Markdown files in `crates/oxibot-agent/skills/`.

1. Create `crates/oxibot-agent/skills/your-skill.md`:

```markdown
---
name: your-skill
description: Brief description of what this skill does
version: "1.0"
---

# Your Skill

Instructions for the agent on how to use this skill.

## When to activate

- User asks about X
- User wants to do Y

## How to respond

1. Step one
2. Step two
3. Step three
```

2. The skill is automatically loaded by the agent at startup
3. Test by asking the agent something that should trigger the skill

---

## Commit Convention

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

### Types

| Type | Description |
|------|-------------|
| `feat` | New feature |
| `fix` | Bug fix |
| `docs` | Documentation only |
| `style` | Formatting, no code change |
| `refactor` | Code change, no new feature or fix |
| `perf` | Performance improvement |
| `test` | Adding or fixing tests |
| `chore` | Build process, dependencies, CI |

### Scopes

`core`, `agent`, `providers`, `channels`, `cron`, `cli`, `bridge`, `docker`

### Examples

```
feat(channels): add Matrix channel support
fix(agent): prevent infinite loop on empty response
docs(readme): add WhatsApp setup guide
test(providers): add unit tests for Groq provider
chore(docker): update base image to debian bookworm
```

---

## Questions?

Open a [Discussion](https://github.com/DioCrafts/OxiBot/discussions) on GitHub or reach out to the maintainers.

<p align="center">
  <sub>Thank you for helping make OxiBot better! 🦀</sub>
</p>
