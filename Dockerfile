# ── Stage 1: Build sandbox server ──
FROM rust:1.83-slim AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY server/ server/
COPY lib/seccomp_redbear/ lib/seccomp_redbear/
RUN cargo build --release -p redbear-sandbox

# ── Stage 2: Build seccomp shared libs ──
FROM rust:1.83-slim AS seccomp
RUN apt-get update && apt-get install -y --no-install-recommends libseccomp-dev
COPY lib/seccomp_redbear/ /build/
RUN cargo build --release --manifest-path /build/Cargo.toml --features python3 && \
    mv /build/target/release/libsandbox.so /out/libpython.so
RUN cargo build --release --manifest-path /build/Cargo.toml --features nodejs && \
    mv /build/target/release/libsandbox.so /out/libnodejs.so

# ── Stage 3: Runtime ──
FROM debian:bookworm-slim
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        python3 python3-pip nodejs npm libseccomp2 ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

# Seccomp libs
COPY --from=seccomp /out/libpython.so /usr/local/lib/
COPY --from=seccomp /out/libnodejs.so /usr/local/lib/

# Server binary
COPY --from=builder /build/target/release/redbear-sandbox /usr/local/bin/redbear-sandbox

# Runtime assets
COPY server/config.toml /etc/sandbox/config.toml
COPY prescript.py /usr/local/share/sandbox/
COPY dependencies/ /usr/local/share/sandbox/dependencies/
RUN mkdir -p /usr/local/share/sandbox/tmp && chmod 1777 /usr/local/share/sandbox/tmp

RUN useradd -u 65537 sandbox
ENV CONFIG_PATH=/etc/sandbox/config.toml

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -sf http://127.0.0.1:8194/health || exit 1

EXPOSE 8194
CMD ["redbear-sandbox"]
