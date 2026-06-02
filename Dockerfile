FROM rust:1.96-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential && \
    rm -rf /var/lib/apt/lists/*

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
