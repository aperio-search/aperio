FROM rust:1.95-slim-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release && \
    cp target/release/aperio /aperio

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /aperio /aperio

ENV DATA_DIR=/data
ENV CONFIG_FILE=/data/config.toml
VOLUME /data

EXPOSE 3000

CMD ["/aperio"]
