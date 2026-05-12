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

# Non-root user. UID 1000 matches Railway's default volume-permissions.
RUN useradd --create-home --uid 1000 pankosmia

# Layout under /app: binary + baked assets read by the server.
COPY --from=build /build/target/release/pankosmia_docker /app/bin/pankosmia_docker
COPY app_resources /app/app_resources
COPY catalog /app/catalog

# The workspace dir is volume-mounted at runtime. Pre-create + chown
# so the non-root user can write on first boot if no volume is
# attached (handy for `docker run` smoke tests). On Railway the
# attached volume takes precedence and inherits its own ownership.
RUN mkdir -p /data && chown -R pankosmia:pankosmia /data /app

USER pankosmia

EXPOSE 19119

ENV ROCKET_ADDRESS=0.0.0.0 \
    ROCKET_PORT=19119 \
    APP_RESOURCES_DIR=/app/app_resources/ \
    PANKOSMIA_CATALOG_PATH=/app/catalog/languages.yaml

# `PORT` from the PaaS (Railway, Fly.io, etc.) is bridged to
# ROCKET_PORT inside main(); if neither is set the server defaults
# to ROCKET_PORT=19119 via the EXPOSE directive's hint and Rocket's
# own default.

ENTRYPOINT ["/app/bin/pankosmia_docker"]
CMD ["/data"]
