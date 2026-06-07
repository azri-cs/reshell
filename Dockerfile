# Stage 1: Build
FROM rust:1.80-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ ./src/
COPY benches/ ./benches/

RUN cargo build --release && \
    strip target/release/rsh

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/rsh /usr/local/bin/rsh

# Create non-root user for security
RUN useradd --create-home --shell /bin/sh rshuser
USER rshuser
WORKDIR /home/rshuser

ENTRYPOINT ["rsh"]
CMD ["mcp"]
