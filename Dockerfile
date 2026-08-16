FROM node:20-alpine AS frontend
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY tsconfig.json tsconfig.app.json tsconfig.node.json vite.config.ts components.json index.html ./
COPY src ./src
RUN npm run build

# Build the headless web server. It is its own crate under
# crates/web_server/ — no Tauri/GTK/webklt dependency is pulled in,
# so the build runs on plain Debian without libwebkit2gtk-4.1-dev.
FROM rust:1.91-bookworm AS web-builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app/src-tauri/crates/web_server
COPY src-tauri/crates/web_server/Cargo.toml ./
# `Cargo.toml` references the workspace; fetch all transitive deps in the
# workspace context so cargo can reuse them across rebuilds.
COPY src-tauri/crates ./crates
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ../../
COPY src/lib ../../src/lib
RUN mkdir -p src templates && \
    echo "// stub src/lib/source-watch-defaults.json sentinel" > /dev/null
RUN cargo build --release --bin llm-wiki-web
RUN strip target/release/llm-wiki-web

# `debian:bookworm-slim` ships `wget` and the standard glibc runtime the
# produced binary was linked against. `ca-certificates` is required for
# outbound HTTPS calls to LLM providers.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    wget \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=web-builder /app/src-tauri/crates/web_server/target/release/llm-wiki-web /usr/local/bin/llm-wiki-web
COPY --from=frontend /app/dist /app/dist
ENV LLM_WIKI_DATA_DIR=/data
ENV LLM_WIKI_DIST_DIR=/app/dist
ENV LLM_WIKI_BIND_HOST=0.0.0.0
ENV LLM_WIKI_PORT=8080
EXPOSE 8080
VOLUME ["/data"]
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
  CMD wget -q -O - http://127.0.0.1:8080/api/v1/health || exit 1
ENTRYPOINT ["/usr/local/bin/llm-wiki-web"]