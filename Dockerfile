# Multi-stage build for pankosmia-docker.
#
# Stage 1: build the binary against a pinned Rust toolchain.
# Stage 2: minimal Debian-slim runtime image with the binary + the
#          baked assets the server reads on boot.
#
# Runtime image layout:
#   /app/bin/pankosmia_docker        ← the binary
#   /app/app_resources/              ← APP_RESOURCES_DIR (config + templates)
#   /app/catalog/languages.yaml      ← initial catalog (overridable via PANKOSMIA_CATALOG_PATH)
#   /data                            ← workspace dir; volume-mounted in production

# --- Planner stage ------------------------------------------------
FROM rust:1.90-slim-bookworm AS planner
RUN cargo install cargo-chef --locked
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

# --- Build stage --------------------------------------------------
FROM rust:1.90-slim-bookworm AS build
RUN cargo install cargo-chef --locked
WORKDIR /build

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       pkg-config \
       libssl-dev \
       cmake \
       zlib1g-dev \
       ca-certificates \
       perl \
       perl-modules \
    && rm -rf /var/lib/apt/lists/*

# Cook dependencies (cached until Cargo.toml/Cargo.lock change).
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Build only the application code (fast — deps already compiled).
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

# --- Runtime stage ------------------------------------------------
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates \
       libssl3 \
       zlib1g \
    && rm -rf /var/lib/apt/lists/*

# Non-root user. The container starts as root so the entrypoint can
# fix volume ownership (Railway mounts /data root-owned), then drops
# to this user via setpriv before exec'ing the binary.
RUN useradd --create-home --uid 1000 pankosmia

# Layout under /app: binary + baked assets + entrypoint.
COPY --from=build /build/target/release/pankosmia_docker /app/bin/pankosmia_docker
COPY app_resources /app/app_resources
COPY catalog /app/catalog
COPY scripts/entrypoint.sh /app/bin/entrypoint.sh
RUN chmod +x /app/bin/entrypoint.sh

# Pre-create /data so `docker run` without a volume mount still
# works for smoke tests.
RUN mkdir -p /data && chown -R pankosmia:pankosmia /data /app

# Intentionally no `USER` directive — see entrypoint script.

EXPOSE 19119

ENV ROCKET_ADDRESS=0.0.0.0 \
    ROCKET_PORT=19119 \
    APP_RESOURCES_DIR=/app/app_resources/ \
    PANKOSMIA_CATALOG_PATH=/app/catalog/languages.yaml \
    PANKOSMIA_I18N_TEMPLATE=/app/app_resources/templates/i18n.json

# `PORT` from the PaaS (Railway, Fly.io, etc.) is bridged to
# ROCKET_PORT inside main(); the EXPOSE directive's 19119 is the
# fallback when neither is set.

ENTRYPOINT ["/app/bin/entrypoint.sh"]
CMD ["/data"]
