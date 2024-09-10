FROM rust:1.76.0-alpine AS builder
WORKDIR /app
RUN apk add musl-dev openssl-dev
ENV OPENSSL_DIR=/usr
COPY . .
RUN cargo install sqlx-cli --no-default-features --features sqlite
ENV DATABASE_URL=sqlite:config/db.sqlite3
ENV DB_PATH=/config/db.sqlite3
RUN sqlx migrate run \
  && cargo build --release

FROM scratch
WORKDIR /
COPY --from=builder /app/target/release/floundr .
COPY --from=builder /app/database/db.sqlite3 ./db.sqlite3
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
ENV DB_PATH=/db.sqlite3
EXPOSE 8080
EXPOSE 443

# you will need to mount the config directory to /config
# and provide an ssl cert/key pair
ENTRYPOINT ["./floundr", "--port", "8080", "--ssl", "--https-port", "443", "--cert-path", "/config/server.crt", "--key-path", "/config/floundr-key.pem"]
