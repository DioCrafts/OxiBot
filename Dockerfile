# ─────────────────────────────────────────────
# Oxibot — multi-stage Docker build
# ─────────────────────────────────────────────
# Stage 1: build the Rust binary
# Stage 2: minimal runtime image
# ─────────────────────────────────────────────

# ── Builder ──────────────────────────────────
FROM rust:1.84-bookworm AS builder

WORKDIR /build

# Copy workspace manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY crates/oxibot-core/Cargo.toml crates/oxibot-core/Cargo.toml
COPY crates/oxibot-agent/Cargo.toml crates/oxibot-agent/Cargo.toml
COPY crates/oxibot-providers/Cargo.toml crates/oxibot-providers/Cargo.toml
COPY crates/oxibot-channels/Cargo.toml crates/oxibot-channels/Cargo.toml
COPY crates/oxibot-cron/Cargo.toml crates/oxibot-cron/Cargo.toml
COPY crates/oxibot-cli/Cargo.toml crates/oxibot-cli/Cargo.toml

# Create stub src files so cargo can resolve the workspace
RUN mkdir -p crates/oxibot-core/src && echo "pub fn stub(){}" > crates/oxibot-core/src/lib.rs && \
    mkdir -p crates/oxibot-agent/src && echo "pub fn stub(){}" > crates/oxibot-agent/src/lib.rs && \
    mkdir -p crates/oxibot-providers/src && echo "pub fn stub(){}" > crates/oxibot-providers/src/lib.rs && \
    mkdir -p crates/oxibot-channels/src && echo "pub fn stub(){}" > crates/oxibot-channels/src/lib.rs && \
    mkdir -p crates/oxibot-cron/src && echo "pub fn stub(){}" > crates/oxibot-cron/src/lib.rs && \
    mkdir -p crates/oxibot-cli/src && echo "fn main(){}" > crates/oxibot-cli/src/main.rs

# Pre-build dependencies (cached unless Cargo.toml/lock change)
RUN cargo build --release --features "telegram,discord,whatsapp,slack,email" 2>/dev/null || true

# Copy full source
COPY crates/ crates/

# Touch all source files to invalidate the stub builds
RUN find crates -name "*.rs" -exec touch {} +

# Build the real binary with all channel features
RUN cargo build --release --features "telegram,discord,whatsapp,slack,email"

# ── Bridge (Node.js) ─────────────────────────
FROM node:20-bookworm-slim AS bridge-builder

WORKDIR /bridge
COPY bridge/package.json bridge/package-lock.json* ./
RUN npm install --ignore-scripts
COPY bridge/ ./
RUN npm run build

# ── Runtime ──────────────────────────────────
FROM debian:bookworm-slim AS runtime

# Install Node.js 20 for the WhatsApp bridge sidecar
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        tmux \
        gnupg \
    && mkdir -p /etc/apt/keyrings \
    && curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key \
       | gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg \
    && echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_20.x nodistro main" \
       > /etc/apt/sources.list.d/nodesource.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends nodejs \
    && apt-get purge -y gnupg \
    && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

# Create oxibot user
RUN useradd -m -s /bin/bash oxibot

# Copy binary from builder
COPY --from=builder /build/target/release/oxibot /usr/local/bin/oxibot

# Copy bundled skills
COPY --from=builder /build/crates/oxibot-agent/skills/ /usr/share/oxibot/skills/

# Copy WhatsApp bridge
COPY --from=bridge-builder /bridge/dist/ /usr/share/oxibot/bridge/dist/
COPY --from=bridge-builder /bridge/node_modules/ /usr/share/oxibot/bridge/node_modules/
COPY --from=bridge-builder /bridge/package.json /usr/share/oxibot/bridge/package.json

# Create config and workspace directories
RUN mkdir -p /home/oxibot/.oxibot /home/oxibot/workspace && \
    chown -R oxibot:oxibot /home/oxibot

USER oxibot
WORKDIR /home/oxibot

# Gateway default port + bridge WS port
EXPOSE 18790 3001

ENTRYPOINT ["oxibot"]
CMD ["status"]
