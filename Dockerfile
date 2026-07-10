# ── Stage 1: Build sandbox server ──
FROM rust:1.88-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libseccomp-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy workspace manifest and all source (lock file enables reproducible builds)
COPY Cargo.toml Cargo.lock ./
COPY server/ server/
COPY lib/seccomp_redbear/ lib/seccomp_redbear/
COPY prescript.py prescript.js ./

# Build server (uses workspace resolver)
RUN cargo build --release -p redbear-sandbox

# ── Stage 2: Build seccomp shared libs ──
FROM rust:1.88-slim-bookworm AS seccomp

RUN apt-get update && apt-get install -y --no-install-recommends \
    libseccomp-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY lib/seccomp_redbear/ ./

RUN cargo build --release --features python3 && \
    mv target/release/libsandbox.so /libpython.so

RUN cargo clean && \
    cargo build --release --features nodejs && \
    mv target/release/libsandbox.so /libnodejs.so

# ── Stage 3: Runtime ──
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 python3-pip nodejs npm libseccomp2 ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

# Seccomp shared libs (loaded at runtime by prescript)
COPY --from=seccomp /libpython.so /usr/local/share/sandbox/libpython.so
COPY --from=seccomp /libnodejs.so /usr/local/share/sandbox/libnodejs.so

# Server binary
COPY --from=builder /build/target/release/redbear-sandbox /usr/local/bin/redbear-sandbox

# Config and runtime assets
COPY server/config.toml /etc/sandbox/config.toml
COPY prescript.py prescript.js /usr/local/share/sandbox/
COPY dependencies/ /usr/local/share/sandbox/dependencies/
COPY script/ /usr/local/share/sandbox/script/

RUN mkdir -p /usr/local/share/sandbox/tmp && chmod 1777 /usr/local/share/sandbox/tmp

# Install Node.js koffi FFI library (required by prescript.js for seccomp)
RUN cd /usr/local/share/sandbox && npm install koffi

# Sandbox user for privilege dropping
RUN useradd -u 65537 sandbox

ENV CONFIG_PATH=/etc/sandbox/config.toml

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://127.0.0.1:8194/health || exit 1

EXPOSE 8194
CMD ["redbear-sandbox"]
