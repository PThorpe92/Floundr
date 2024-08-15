FROM rust:1.79.0-alpine AS builder
WORKDIR /app
COPY . .
RUN apk add musl-dev openssl-dev
ENV OPENSSL_DIR=/usr
RUN cargo install sqlx-cli --no-default-features --features sqlite
RUN mkdir -p database && touch database/db.sqlite3
ENV DATABASE_URL=sqlite:database/db.sqlite3
RUN sqlx migrate run
RUN cargo build --release

FROM scratch
WORKDIR /
ENV DATABASE_URL=sqlite:/db.sqlite3
COPY --from=builder /app/target/release/floundr .
COPY --from=builder /app/database/db.sqlite3 /db.sqlite3
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
EXPOSE 8080
ENTRYPOINT ["./floundr", "--port", "8080", "--new-repo", "test_repo", "--public", "false"]
