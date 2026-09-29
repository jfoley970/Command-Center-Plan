# cc-server image: the shared backend plus the built web UI.
#   docker build -t command-center-server .
# See deploy/README.md for running it behind Caddy and the auth service.

FROM node:22-bookworm-slim AS web
WORKDIR /src
COPY package.json package-lock.json ./
RUN npm ci
COPY index.html tsconfig.json tsconfig.node.json vite.config.ts ./
COPY src ./src
RUN npm run build

FROM rust:1-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# Only the manifest is needed so Cargo can read the workspace; the desktop app is not built.
COPY src-tauri/Cargo.toml ./src-tauri/Cargo.toml
RUN mkdir -p src-tauri/src && echo "fn main() {}" > src-tauri/src/main.rs && touch src-tauri/src/lib.rs \
 && cargo build --release -p cc-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home /data cc
COPY --from=server /src/target/release/cc-server /usr/local/bin/cc-server
COPY --from=web /src/dist /srv/web
ENV CC_DATA_DIR=/data CC_WEB_DIR=/srv/web CC_BIND=0.0.0.0:8484
VOLUME /data
USER cc
EXPOSE 8484
HEALTHCHECK --interval=30s --timeout=5s CMD curl -fsS http://127.0.0.1:8484/api/health || exit 1
CMD ["cc-server"]
