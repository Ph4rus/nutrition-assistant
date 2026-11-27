FROM rust:1.100-alpine3.22 AS builder

RUN apk add --no-cache musl-dev
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release

FROM ghcr.io/home-assistant/base:3.24

COPY --from=builder /build/target/release/nutrition-assistant /usr/bin/nutrition-assistant

COPY run.sh /
RUN chmod a+x /run.sh

CMD [ "/run.sh" ]
