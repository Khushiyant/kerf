# Multi-stage build of the kerf-serve platform binary.
FROM rust:1.90-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p kerf-api --bin kerf-serve --features postgres

# Minimal, non-root runtime (distroless cc for the glibc-linked binary).
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/kerf-serve /usr/local/bin/kerf-serve
ENV KERF_ADDR=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/kerf-serve"]
