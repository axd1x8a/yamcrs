FROM rust:latest AS builder

WORKDIR /yamcrs

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

COPY --from=builder /lib/x86_64-linux-gnu/libgcc_s.so.1 /lib/x86_64-linux-gnu/libgcc_s.so.1
COPY --from=builder /lib/x86_64-linux-gnu/libm.so.6       /lib/x86_64-linux-gnu/libm.so.6
COPY --from=builder /lib/x86_64-linux-gnu/libc.so.6       /lib/x86_64-linux-gnu/libc.so.6
COPY --from=builder /lib64/ld-linux-x86-64.so.2           /lib64/ld-linux-x86-64.so.2

CMD ["/yamcrs/yamcrs"]
