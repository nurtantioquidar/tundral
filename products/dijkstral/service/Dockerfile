# The service image (AGENTS.md). The build context is products/dijkstral, so the path
# dependencies on the CLI's crates resolve; from the repository root:
#   docker build -f products/dijkstral/service/Dockerfile products/dijkstral
FROM rust:1.96.1-slim-trixie AS build
WORKDIR /src
COPY cli/Cargo.toml cli/
COPY cli/crates cli/crates
COPY service/Cargo.toml service/Cargo.lock service/
COPY service/src service/src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/service/target \
    cd service \
    && cargo build --release --locked \
    && cp target/release/dijkstral-service /usr/local/bin/

FROM debian:trixie-slim
# git fetches repositories (src/git.rs); ca-certificates verifies GitHub for it and the API client.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/dijkstral-service /usr/local/bin/
# Non-root; a new named volume at /data takes this ownership.
RUN install -d -o 65532 -g 65532 -m 0700 /data
USER 65532:65532
ENV DIJKSTRAL_SERVICE_DATA=/data \
    DIJKSTRAL_SERVICE_LISTEN=0.0.0.0:8095
EXPOSE 8095
ENTRYPOINT ["dijkstral-service"]
CMD ["serve"]
