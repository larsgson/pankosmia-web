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

# --- Build stage --------------------------------------------------
FROM rust:1.90-slim-bookworm AS build
WORKDIR /build

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       pkg-config \
       libssl-dev \
       cmake \
       zlib1g-dev \
       ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Fetch deps separately so the registry layer caches across most
# code changes. Compile happens in the next layer with the real
# sources — the stub-source dep-cache trick is too fragile with
# Cargo's fingerprint tracking when the crate has both [[bin]] and
# [lib] targets.
COPY Cargo.toml Cargo.lock ./
RUN cargo fetch --locked

COPY src ./src
RUN cargo build --release --locked --offline

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
    PANKOSMIA_CATALOG_PATH=/app/catalog/languages.yaml

# `PORT` from the PaaS (Railway, Fly.io, etc.) is bridged to
# ROCKET_PORT inside main(); the EXPOSE directive's 19119 is the
# fallback when neither is set.

ENTRYPOINT ["/app/bin/entrypoint.sh"]
CMD ["/data"]
