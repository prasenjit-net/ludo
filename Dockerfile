# syntax=docker/dockerfile:1

# ---- build: wasm game (trunk) + server with the game embedded -----------------
# trunk's release binaries need glibc 2.39+, so build on Debian 13 (and run on the matching base).
FROM rust:1-trixie AS build

ARG TRUNK_VERSION=0.22.1

RUN rustup target add wasm32-unknown-unknown
# trunk publishes Linux binaries for x86_64 and aarch64; `uname -m` uses the same names.
RUN curl -fsSL "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-$(uname -m)-unknown-linux-gnu.tar.gz" \
    | tar -xz -C /usr/local/bin

WORKDIR /app
COPY . .

# trunk downloads wasm-bindgen and wasm-opt on first use; the cache mounts make rebuilds quick.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    --mount=type=cache,target=/root/.cache/trunk \
    cd ludo-game && trunk build --release

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p server \
 && cp target/release/server /ludo-server

# ---- runtime: one static-ish binary, no shell ----------------------------------
FROM gcr.io/distroless/cc-debian13:nonroot

COPY --from=build /ludo-server /ludo-server

ENV LUDO_ADDR=0.0.0.0:3000
EXPOSE 3000
ENTRYPOINT ["/ludo-server"]
