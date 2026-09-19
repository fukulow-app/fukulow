# Builds the `fukulow` binary for compose.yaml. Base images are pinned by digest;
# change the tag and the digest together.
FROM rust:1.98.1-slim-bookworm@sha256:8d50cf1cfb8929fbf0c2c1bf654c1f21693862c1facf42d2926d7fed716422ac AS build
WORKDIR /src
# rust-toolchain.toml also asks for rustfmt and clippy, which a release build does
# not use; naming the installed toolchain keeps rustup from downloading them.
ENV RUSTUP_TOOLCHAIN=1.98.1 \
    SQLX_OFFLINE=true \
    CARGO_TERM_COLOR=never
COPY . .
RUN cargo build --release --locked -p app --bin fukulow

FROM debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171
# ca-certificates: a DATABASE_URL may point at a PostgreSQL server over TLS.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin fukulow
COPY --from=build /src/target/release/fukulow /usr/local/bin/fukulow
USER 10001
ENV FUKULOW_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
CMD ["fukulow"]
