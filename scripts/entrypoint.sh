#!/bin/sh
# Container entrypoint.
#
# Railway (and other bind-mount-based volume providers) mount the
# workspace as root-owned. The server runs as the non-root
# `pankosmia` user (UID 1000) and needs to write into it. This
# script handles that, while still degrading cleanly for local
# `docker run` without a volume.
#
# If we're root at boot:
#   1. Take ownership of /data (idempotent; cheap on already-owned).
#   2. Drop to UID 1000 via setpriv before exec'ing the binary.
# Otherwise (already a non-root user), just exec.
set -e

if [ "$(id -u)" = "0" ]; then
    chown -R pankosmia:pankosmia /data 2>/dev/null || true
    exec setpriv \
        --reuid=pankosmia \
        --regid=pankosmia \
        --init-groups \
        /app/bin/pankosmia_docker "$@"
fi

exec /app/bin/pankosmia_docker "$@"
