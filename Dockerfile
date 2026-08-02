FROM node:20-alpine AS frontend
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY tsconfig.json tsconfig.app.json tsconfig.node.json vite.config.ts components.json index.html ./
COPY src ./src
RUN npm run build

# Stage dependency resolution once so the (slow) incremental rebuild only
# pays the cost of source-file changes. Cargo's incremental cache is
# keyed off `Cargo.toml` / `Cargo.lock`, so swapping those busts the
# cache the same way as touching a Rust source file.
FROM rust:1.91-bookworm AS deps
# Tauri's Linux dependency tree pulls in webkit/gtk/glib via pkg-config;
# installing them on Debian Bookworm via the standard apt packages works
# reliably across architectures (the musl/Alpine variant ships older
# libs and produces a binary that won't link cleanly for headless Rust
# servers that don't actually exercise the GUI stack).
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    libgtk-3-dev \
    libsoup-3.0-dev \
    libwebkit2gtk-4.1-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    protobuf-compiler \
    libprotobuf-dev \
    liblzma-dev \
    libbz2-dev \
    zlib1g-dev \
    libpango1.0-dev \
    libharfbuzz-dev \
    libgdk-pixbuf-2.0-dev \
    libcairo2-dev \
    libcairo-gobject2 \
    gettext \
    libdbus-1-dev \
    libatk1.0-dev \
    libatk-bridge2.0-dev \
    libatspi2.0-dev \
    gcc \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app/src-tauri
COPY src-tauri/Cargo.toml src-tauri/Cargo.lock ./
COPY src-tauri/build.rs ./
COPY src-tauri/windows-app-manifest.xml ./
COPY src-tauri/icons ./icons
COPY src-tauri/capabilities ./capabilities
COPY src-tauri/src ./src
COPY src-tauri/tauri.conf.json ./
# `commands/file_sync.rs` embeds the default source-watch config via
# `include_str!("../../../src/lib/source-watch-defaults.json")`. The
# cargo manifest dir during the build is `/app/src-tauri`, so the
# resolved path is `/app/src/lib/...` — surface that here.
COPY src/lib /app/src/lib
RUN echo "fn main() {}" > src/main.rs \
    && echo "fn main() {}" > src/bin/llm-wiki-web.rs \
    && cargo build --release --features web --bin llm-wiki-web

FROM rust:1.91-bookworm AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    libgtk-3-dev \
    libsoup-3.0-dev \
    libwebkit2gtk-4.1-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    protobuf-compiler \
    libprotobuf-dev \
    liblzma-dev \
    libbz2-dev \
    zlib1g-dev \
    libpango1.0-dev \
    libharfbuzz-dev \
    libgdk-pixbuf-2.0-dev \
    libcairo2-dev \
    libcairo-gobject2 \
    gettext \
    libdbus-1-dev \
    libatk1.0-dev \
    libatk-bridge2.0-dev \
    libatspi2.0-dev \
    gcc \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY src-tauri ./src-tauri
COPY src/lib /app/src/lib
COPY --from=deps /app/src-tauri/target /app/src-tauri/target
COPY --from=frontend /app/dist ./dist
RUN cd src-tauri && \
    cargo build --release --features web --bin llm-wiki-web && \
    strip target/release/llm-wiki-web

# Debian slim ships `wget` via the `wget` package (busybox is not the
# default shell), `ca-certificates` for outbound HTTPS, and the standard
# glibc runtime that the produced binary was linked against.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libgtk-3-0 \
    libsoup-3.0-0 \
    libwebkit2gtk-4.1-0 \
    libayatana-appindicator3-0.1 \
    librsvg2-2 \
    libpango-1.0-0 \
    libharfbuzz0b \
    libgdk-pixbuf-2.0-0 \
    libcairo2 \
    libdbus-1-3 \
    libatk1.0-0 \
    libatk-bridge2.0-0 \
    libatspi2.0-0 \
    libssl3 \
    wget \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=runtime /app/src-tauri/target/release/llm-wiki-web /usr/local/bin/llm-wiki-web
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
