FROM rust:1.93-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock* rust-toolchain.toml ./
COPY crates/ crates/

RUN cargo build --release -p codex-shim && \
    cp target/release/codex-shim /usr/local/bin/

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/codex-shim /usr/local/bin/codex-shim

EXPOSE 8787

ENTRYPOINT ["codex-shim"]
CMD ["--config", "/etc/codex-shim/config.yaml"]
