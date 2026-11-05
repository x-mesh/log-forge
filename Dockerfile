FROM rust:1.99.1-slim-trixie AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY data ./data
COPY config ./config
RUN cargo build --release --locked

FROM debian:trixie-slim
WORKDIR /app
RUN mkdir -p /app/logs && chown 65532:65532 /app/logs
COPY --from=build /build/target/release/log-forge /usr/local/bin/log-forge
RUN ln -s log-forge /usr/local/bin/log-generator
COPY config /app/config
COPY adapters /app/adapters
COPY demo_patterns /app/demo_patterns
COPY error_patterns /app/error_patterns
COPY error_scenarios /app/error_scenarios
USER 65532:65532
ENTRYPOINT ["log-forge"]
CMD ["start", "--output", "console", "--no-monitor"]
