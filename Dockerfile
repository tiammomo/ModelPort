# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.96.0
ARG MODELPORT_VERSION=0.1.2
ARG MODELPORT_SOURCE_REVISION=unknown
ARG MODELPORT_SOURCE_STATE=unknown
ARG MODELPORT_BUILD_DATE=unknown
FROM rust:${RUST_VERSION}-bookworm AS builder

ARG MODELPORT_SOURCE_REVISION
ARG MODELPORT_SOURCE_STATE
ENV MODELPORT_BUILD_REVISION=${MODELPORT_SOURCE_REVISION}
ENV MODELPORT_BUILD_SOURCE_STATE=${MODELPORT_SOURCE_STATE}

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY crates ./crates
# Embedded at compile time via include_str! (src/model_catalog.rs,
# src/runtime_adapter.rs); a missing directory fails `cargo build`.
COPY resources ./resources

RUN cargo build --release --locked -p model-port

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*

RUN useradd --system --home /nonexistent --shell /usr/sbin/nologin modelport
RUN mkdir -p /data /config \
  && chown -R modelport:modelport /data /config

COPY --from=builder /app/target/release/model-port /usr/local/bin/model-port

# Keep source metadata after dependency and binary layers so a new commit label
# does not invalidate the slow apt or Rust build cache.
ARG MODELPORT_SOURCE_REVISION
ARG MODELPORT_SOURCE_STATE
ARG MODELPORT_VERSION
ARG MODELPORT_BUILD_DATE
LABEL org.opencontainers.image.title="ModelPort" \
      org.opencontainers.image.description="Self-hosted multi-protocol model gateway" \
      org.opencontainers.image.source="https://github.com/tiammomo/ModelPort" \
      org.opencontainers.image.revision="$MODELPORT_SOURCE_REVISION" \
      org.opencontainers.image.version="$MODELPORT_VERSION" \
      org.opencontainers.image.created="$MODELPORT_BUILD_DATE" \
      org.opencontainers.image.licenses="MIT" \
      io.modelport.source-state="$MODELPORT_SOURCE_STATE"

USER modelport

ENV MODELPORT_BIND=0.0.0.0:38082
ENV MODELPORT_STATE_DIR=/data
ENV MODELPORT_CONFIG=/config/config.toml
ENV RUST_LOG=model_port=info,tower_http=info

EXPOSE 38082
VOLUME ["/data"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -fsS http://127.0.0.1:38082/livez >/dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/model-port"]
