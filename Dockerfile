FROM rust:1.77-alpine AS builder

WORKDIR /app
RUN apk add musl-dev openssl-dev
ENV OPENSSL_DIR=/usr
COPY oci_rs.db ./
ENV DATABASE_URL=sqlite://oci_rs.db
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release


FROM scratch as runtime
WORKDIR /
COPY oci_rs.db ./
COPY --from=builder /app/target/release/oci_rs ./
ENTRYPOINT ["./oci_rs"]
