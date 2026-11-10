FROM rust:1.95-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p messenger-headless

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 node \
    && mkdir -m 0700 /data \
    && chown node:node /data
COPY --from=build /src/target/release/messenger-headless /usr/local/bin/messenger-headless
USER node
ENTRYPOINT ["messenger-headless"]
