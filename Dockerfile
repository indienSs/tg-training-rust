FROM rust:1.99-slim AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends tzdata ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home bot
WORKDIR /app
COPY --from=builder /app/target/release/tg-training-bot /usr/local/bin/tg-training-bot
RUN mkdir -p /app/data && chown bot:bot /app/data
USER bot
ENV TZ=Europe/Moscow
VOLUME /app/data
ENTRYPOINT ["tg-training-bot"]
