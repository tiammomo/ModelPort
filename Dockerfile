# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.96.0
FROM rust:${RUST_VERSION}-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*

RUN useradd --system --home /nonexistent --shell /usr/sbin/nologin modelport
RUN mkdir -p /data /config \
  && chown -R modelport:modelport /data /config

COPY --from=builder /app/target/release/model-port /usr/local/bin/model-port

USER modelport

ENV MODELPORT_BIND=0.0.0.0:17878
ENV MODELPORT_STATE_DIR=/data
ENV MODELPORT_CONTROL_STORE=/data/control-plane.json
ENV MODELPORT_CONFIG=/config/config.toml
ENV RUST_LOG=model_port=info,tower_http=info

EXPOSE 17878
VOLUME ["/data"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -fsS http://127.0.0.1:17878/livez >/dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/model-port"]
