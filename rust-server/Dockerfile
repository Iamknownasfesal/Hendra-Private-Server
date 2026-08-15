# The Rust server, built once and shipped without its toolchain.
#
# Two stages because a build image is a gigabyte of compiler that a running server has no use for.
# The result carries the binaries, the migrations they apply, the behaviours and maps, and the game
# content they are all meaningless without.
#
# Built from the repository root rather than from `rust-server/`, because the game content lives in
# `godot-client/assets/xml` and a build context rooted at the server cannot see it:
#
#     python3 godot-client/tools/extract_assets.py
#     docker build -f rust-server/Dockerfile -t hendra .
#
# The extractor is not optional. `godot-client/assets/xml` is generated out of the ActionScript
# client and is not in version control, so a fresh clone has none of it and the COPY below fails
# rather than producing an image that cannot start.

FROM rust:1.97-slim AS build
WORKDIR /build

# Dependencies first, on their own layer. Source changes far more often than Cargo.toml does, so
# this is what keeps an ordinary rebuild from recompiling every crate in the tree.
COPY rust-server/Cargo.toml rust-server/Cargo.lock ./
COPY rust-server/crates ./crates
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
COPY --chown=hendra:hendra rust-server/crates/store/migrations ./migrations
COPY --chown=hendra:hendra rust-server/content ./content
COPY --chown=hendra:hendra godot-client/assets/xml ./content/xml

# What the app server reads for the character-select screen. `content/xmls` beside it is the same
# content as the original C# server reads; this image serves the client's own copy, which carries
# several hundred objects more, so it is named rather than left to the default.
ENV HENDRA_CONTENT=/home/hendra/content/xml

# The game socket. The app server's port is set by HENDRA_APP_ADDRESS and is not exposed here,
# because it belongs behind whatever terminates TLS.
EXPOSE 7777/udp

# Only the object definitions are named, and only to choose the client's copy over `content/xmls`.
# The worlds, the behaviours and the music list are found where the binary looks by default, which
# is inside the `content` directory copied above. The server logs "cannot read content" and exits 1
# when it cannot find the files, so getting this wrong is an image that never starts. Arguments
# passed to `docker run` are appended, and the options are read last-wins, so a caller can still
# point it somewhere else.
ENTRYPOINT ["hendra-server", "--content", "content/xml"]
