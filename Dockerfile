FROM rust:alpine AS builder

WORKDIR /yamcrs

RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY assets ./assets

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/yamcrs/target \
    cargo build --release

RUN --mount=type=cache,target=/yamcrs/target \
    mkdir -p /yamcrs/artifacts \
    && cp /yamcrs/target/release/yamcrs /yamcrs/artifacts/yamcrs

FROM scratch

WORKDIR /yamcrs

COPY --from=builder /yamcrs/artifacts/yamcrs /yamcrs/yamcrs

CMD ["/yamcrs/yamcrs"]
