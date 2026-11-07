# syntax=docker/dockerfile:1
# Build context is the repository root:
#   docker build -f deploy/Dockerfile -t eddy-server .
# Multi-arch:
#   docker buildx build --platform linux/amd64,linux/arm64 -f deploy/Dockerfile -t eddy-server .

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p eddy-server \
 && cp target/release/eddy-server /eddy-server

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /data eddy \
 && mkdir -p /data && chown eddy:eddy /data
COPY --from=build /eddy-server /usr/local/bin/eddy-server
USER eddy
ENV EDDY_DATA_DIR=/data \
    EDDY_BIND=0.0.0.0:8080 \
    RUST_LOG=info
EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["eddy-server"]
CMD ["serve"]
