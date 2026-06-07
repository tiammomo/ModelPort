# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*

RUN useradd --system --home /nonexistent --shell /usr/sbin/nologin modelport

COPY --from=builder /app/target/release/model-port /usr/local/bin/model-port

USER modelport

ENV MODELPORT_BIND=0.0.0.0:17878
ENV RUST_LOG=model_port=info,tower_http=info

EXPOSE 17878

ENTRYPOINT ["/usr/local/bin/model-port"]
