FROM rust:1.76.0-alpine AS builder
WORKDIR /app
RUN apk add musl-dev openssl-dev
ENV OPENSSL_DIR=/usr
COPY . .
RUN cargo install sqlx-cli --no-default-features --features sqlite
RUN mkdir -p database && touch database/db.sqlite3
ENV DATABASE_URL=sqlite:database/db.sqlite3
RUN sqlx migrate run \
  && cargo build --release
RUN floundr new-repo test_repo

FROM scratch
WORKDIR /
COPY --from=builder /app/target/release/floundr .
COPY --from=builder /app/database/db.sqlite3 ./db.sqlite3
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
ENV DB_PATH=/db.sqlite3
EXPOSE 8080
ENTRYPOINT ["./floundr", "--port", "8080"]
