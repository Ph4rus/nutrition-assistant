FROM rust:1.100-alpine3.22 AS builder

RUN apk add --no-cache musl-dev
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release

FROM ghcr.io/home-assistant/base:3.24

ARG BUILD_VERSION=dev
LABEL org.opencontainers.image.title="Nutrition Assistant" \
      org.opencontainers.image.description="Deterministic weekly meal planner for Home Assistant" \
      org.opencontainers.image.source="https://github.com/Ph4rus/nutrition-assistant" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${BUILD_VERSION}"

COPY --from=builder /build/target/release/nutrition-assistant /usr/bin/nutrition-assistant

COPY run.sh /
RUN chmod a+x /run.sh

CMD [ "/run.sh" ]
