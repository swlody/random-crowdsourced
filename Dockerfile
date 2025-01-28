FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release --bin random-crowdsourced

# Create debug info
RUN objcopy --only-keep-debug --compress-debug-sections=zlib /app/target/release/random-crowdsourced /app/target/release/random-crowdsourced.debug
RUN objcopy --strip-debug --strip-unneeded /app/target/release/random-crowdsourced
RUN objcopy --add-gnu-debuglink=/app/target/release/random-crowdsourced.debug /app/target/release/random-crowdsourced

RUN curl -sL https://sentry.io/get-cli | bash

# Upload debug info
RUN mv /app/target/release/random-crowdsourced.debug /app
RUN --mount=type=bind,target=. \
    --mount=type=secret,id=SENTRY_AUTH_TOKEN,env=SENTRY_AUTH_TOKEN \
    sentry-cli debug-files upload --include-sources --org sam-wlody --project random-crowdsourced /app/random-crowdsourced.debug
RUN rm /app/random-crowdsourced.debug

# We do not need the Rust toolchain to run the binary!
FROM debian:bookworm-slim AS runtime
WORKDIR /app
COPY --from=builder /app/target/release/random-crowdsourced /usr/local/bin

COPY assets/static/ /app/assets/static

ENTRYPOINT ["/usr/local/bin/random-crowdsourced"]
