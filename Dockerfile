FROM rust:1.98.0-alpine3.23 AS builder

RUN apk add --no-cache musl-dev
WORKDIR /build

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
COPY web ./web
RUN cargo build --release --locked

FROM alpine:3.23

RUN addgroup -g 10001 -S tpad \
    && adduser -u 10001 -S -D -H -G tpad tpad \
    && mkdir /data \
    && chown tpad:tpad /data

COPY --from=builder /build/target/release/tpad /usr/local/bin/tpad

USER tpad:tpad
EXPOSE 8080
VOLUME ["/data"]
ENV TPAD_DATA_DIR=/data \
    TPAD_LISTEN_ADDR=0.0.0.0:8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=3s --retries=3 \
    CMD wget -q -O /dev/null http://127.0.0.1:8080/healthz || exit 1

ENTRYPOINT ["/usr/local/bin/tpad"]

