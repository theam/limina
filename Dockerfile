# syntax=docker/dockerfile:1.7

# ── Web console ─────────────────────────────────────────────────────────────
FROM node:22-alpine AS web

ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0

WORKDIR /web
RUN corepack enable

# Install from the lockfile first so dependency layers survive source-only changes.
COPY web/package.json web/pnpm-*.yaml ./
RUN pnpm install --frozen-lockfile

COPY web/ ./
RUN pnpm build

# ── Runtime binary ──────────────────────────────────────────────────────────
FROM rust:1.99-bookworm AS build

WORKDIR /src
RUN apt-get update \
    && apt-get install --yes --no-install-recommends cmake clang \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock rustfmt.toml clippy.toml ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin limina \
    && install -m 0755 target/release/limina /usr/local/bin/limina

# ── Provider CLIs ───────────────────────────────────────────────────────────
# Keep the official native provider bundles without their build-time package managers.
# Codex's npm release can precede the Python CLI-bin mirror.
FROM node:22-bookworm-slim AS codex

ARG CODEX_CLI_VERSION=0.164.0

RUN npm install --prefix /providers --omit=dev --no-audit --no-fund \
        "@openai/codex@${CODEX_CLI_VERSION}"

# Preserve companion executables/resources; newer CLI releases use a native bundle layout.
RUN codex_bundle="$(find /providers -type f -name codex-package.json -printf '%h\n')" \
    && test -d "$codex_bundle" \
    && mkdir -p /out \
    && cp -a "$codex_bundle"/. /out/ \
    && /out/bin/codex --version

FROM python:3.13-slim AS claude

ARG CLAUDE_AGENT_SDK_VERSION=0.2.170

RUN pip install --no-cache-dir --target /providers \
        "claude-agent-sdk==${CLAUDE_AGENT_SDK_VERSION}" \
    && install -D -m 0755 /providers/claude_agent_sdk/_bundled/claude /out/claude

# ── Runtime image ───────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends bubblewrap ca-certificates git socat \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /usr/local/bin/limina /usr/local/bin/limina
COPY --from=codex /out/ /opt/codex/
COPY --from=claude /out/claude /usr/local/bin/claude
COPY --from=web /web/dist /app/web

RUN ln -s /opt/codex/bin/codex /usr/local/bin/codex \
    && useradd --create-home --uid 10001 limina \
    && mkdir -p /var/lib/limina/workspaces /var/lib/limina/codex /var/lib/limina/claude \
    && chown -R limina:limina /var/lib/limina
USER limina

ENV LIMINA_DATABASE_URL=sqlite:////var/lib/limina/runtime.db \
    LIMINA_WORKSPACE_ROOT=/var/lib/limina/workspaces \
    CODEX_HOME=/var/lib/limina/codex \
    LIMINA_CLAUDE_CONFIG_DIR=/var/lib/limina/claude \
    LIMINA_CLAUDE_WEAKER_NESTED_SANDBOX=1 \
    LIMINA_WEB_DIR=/app/web \
    PATH=/opt/codex/codex-path:$PATH \
    RUST_LOG=info

EXPOSE 7433
VOLUME ["/var/lib/limina"]

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=6 \
    CMD ["limina", "_healthcheck"]

CMD ["sh", "-c", "limina db upgrade && exec limina serve --host 0.0.0.0 --port 7433"]
