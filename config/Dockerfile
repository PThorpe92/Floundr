# OCI Distribution Conformance Suite
FROM docker.io/golang:1.19.7-alpine3.17 AS builder
ARG VERSION=unknown
ARG GO_PKG=github.com/opencontainers/distribution-spec
RUN apk --update add git make ca-certificates && mkdir -p /go
ARG GO_PKG=https://github.com/opencontainers/distribution-spec.git
WORKDIR /go
RUN git clone ${GO_PKG} src
COPY . .
WORKDIR /go/src/conformance
RUN CGO_ENABLED=0 go test -c -o /conformance.test --ldflags="-X ${GO_PKG}.Version=${VERSION}"
ENTRYPOINT ["/conformance.test"]
