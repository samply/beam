FROM rust:slim-bullseye AS builder
WORKDIR /usr/src/app
RUN cargo search just_to_populate_the_cache >/dev/null
COPY . .
RUN cargo build --bin central --release

FROM debian:bullseye-slim
COPY --from=builder /usr/src/app/target/release/central /usr/local/bin/
ENTRYPOINT [ "/usr/local/bin/central" ]
