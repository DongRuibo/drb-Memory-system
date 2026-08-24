FROM rust:1.97-bookworm AS build

WORKDIR /workspace
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps ./apps
COPY migrations ./migrations
RUN cargo build --locked --release -p memory-service

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /workspace/target/release/memory-service /usr/local/bin/memory-service

EXPOSE 8080
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/memory-service"]
