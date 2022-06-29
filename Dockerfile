FROM rust:slim-bullseye AS builder
WORKDIR /usr/src/app
COPY . .
RUN cargo build --bin central --release

FROM debian:bullseye-slim
COPY --from=builder /usr/src/app/target/release/central /usr/local/bin/
ENTRYPOINT [ "/usr/local/bin/central" ]
