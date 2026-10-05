FROM rust:1.88.0-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked --bin benchmark --bin campaign

FROM debian:bookworm-20250630-slim

RUN groupadd --gid 10001 a220091 \
    && useradd --uid 10001 --gid 10001 --create-home --shell /usr/sbin/nologin a220091 \
    && mkdir -p /var/lib/a220091/campaign /var/lib/a220091/logs /var/lib/a220091/notifications /var/lib/a220091/results \
    && chown -R a220091:a220091 /var/lib/a220091

COPY --from=builder /build/target/release/benchmark /usr/local/bin/a220091-benchmark
COPY --from=builder /build/target/release/campaign /usr/local/bin/a220091-campaign

USER a220091
WORKDIR /var/lib/a220091
VOLUME ["/var/lib/a220091"]
ENTRYPOINT ["/usr/local/bin/a220091-benchmark"]
