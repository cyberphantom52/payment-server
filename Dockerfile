FROM docker.io/library/rust:1.98.1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY migrations ./migrations
COPY provider_migrations ./provider_migrations
RUN cargo build --release --bins --locked -j 2

FROM docker.io/library/debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home payment
COPY --from=build /app/target/release/payment-server /app/target/release/payment-provider /usr/local/bin/
USER payment
CMD ["payment-server"]
