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

RUN cargo build --release --locked -p modelport-ops-agent

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*

RUN useradd --system --home /nonexistent --shell /usr/sbin/nologin modelport-ops \
  && mkdir -p /var/lib/modelport-ops \
  && chown -R modelport-ops:modelport-ops /var/lib/modelport-ops

COPY --from=builder /app/target/release/modelport-ops-agent /usr/local/bin/modelport-ops-agent

ARG MODELPORT_SOURCE_REVISION
ARG MODELPORT_SOURCE_STATE
ARG MODELPORT_VERSION
ARG MODELPORT_BUILD_DATE
LABEL org.opencontainers.image.title="ModelPort Operations Agent" \
      org.opencontainers.image.description="Deterministic read-only operations agent for ModelPort" \
      org.opencontainers.image.source="https://github.com/tiammomo/ModelPort" \
      org.opencontainers.image.revision="$MODELPORT_SOURCE_REVISION" \
      org.opencontainers.image.version="$MODELPORT_VERSION" \
      org.opencontainers.image.created="$MODELPORT_BUILD_DATE" \
      org.opencontainers.image.licenses="MIT" \
      io.modelport.source-state="$MODELPORT_SOURCE_STATE"

USER modelport-ops

ENV MODELPORT_OPS_BIND=0.0.0.0:38083
ENV MODELPORT_OPS_BASE_URL=http://modelport:38082
ENV MODELPORT_OPS_MODE=disabled
ENV MODELPORT_OPS_SPOOL_PATH=/var/lib/modelport-ops/spool.sqlite
ENV RUST_LOG=modelport_ops_agent=info

EXPOSE 38083
VOLUME ["/var/lib/modelport-ops"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
  CMD curl -fsS http://127.0.0.1:38083/readyz >/dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/modelport-ops-agent"]
