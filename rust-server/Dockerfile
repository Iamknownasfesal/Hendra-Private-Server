# The Rust server, built once and shipped without its toolchain.
#
# Two stages because a build image is a gigabyte of compiler that a running server has no use for.
# The result carries three binaries, the migrations they apply, and nothing else.

FROM rust:1.97-slim AS build
WORKDIR /build

# Dependencies first, on their own layer. Source changes far more often than Cargo.toml does, so
# this is what keeps an ordinary rebuild from recompiling every crate in the tree.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked \
        --bin hendra-server --bin hendra-app \
    && mkdir -p /out \
    && cp target/release/hendra-server target/release/hendra-app /out/

FROM debian:bookworm-slim AS runtime

# ca-certificates because the app server talks to Postgres over TLS, and a container without them
# fails at connection time with an error that looks like a network fault.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# An unprivileged user, because nothing here needs root and a compromised game server should not
# be one.
RUN useradd --system --create-home --uid 10001 hendra
USER hendra
WORKDIR /home/hendra

COPY --from=build /out/hendra-server /out/hendra-app /usr/local/bin/
COPY --chown=hendra:hendra crates/store/migrations ./migrations
COPY --chown=hendra:hendra content ./content

# The game socket. The app server's port is set by HENDRA_APP_ADDRESS and is not exposed here,
# because it belongs behind whatever terminates TLS.
EXPOSE 7777/udp

ENTRYPOINT ["hendra-server"]
