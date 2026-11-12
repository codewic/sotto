# Builds the call broker from this repository. Context: the repository root (see docker-compose.yml).
FROM rust:1.99.1-slim-trixie AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p sotto-call-broker

# Distroless, no shell, runs as a non-root user.
FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /src/target/release/sotto-call-broker /usr/local/bin/sotto-call-broker
USER nonroot
ENTRYPOINT ["/usr/local/bin/sotto-call-broker"]
