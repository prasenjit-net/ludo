# Ludo

A 2D Ludo game built with [Bevy](https://bevy.org) (0.20), compiled to WebAssembly and served by a small Rust web server.

This is a proof of concept: the board layout renders, but there is no game logic yet.

## Layout

| Crate | Description |
|---|---|
| [`ludo-game/`](ludo-game/) | The game. A Bevy app compiled to `wasm32-unknown-unknown` and bundled with [Trunk](https://trunkrs.dev). |
| [`server/`](server/) | An axum web server that embeds the game's build output (`ludo-game/dist`) into its binary with `rust-embed`. |

## Prerequisites

- Rust (edition 2024) with the wasm target: `rustup target add wasm32-unknown-unknown`
- Trunk: `cargo install trunk`

## Usage

```sh
make run      # build the game, then serve it at http://127.0.0.1:3000
```

| Target | What it does |
|---|---|
| `make` / `make server` | Build the wasm game, then the server with the game embedded |
| `make run` | Build the game and run the server (set `LUDO_ADDR` to change the address) |
| `make dev` | Hot-reloading `trunk serve` for the game only (port 8080) |
| `make check` | `cargo check` both crates |
| `make clean` | Remove build output |

The game is embedded at compile time, so the server must be rebuilt after the game changes. `make run` does this for you.

## Current state

- Static 15x15 board: cross-shaped track, four yards, start cells, home columns and centre.
- A spinning magenta square in the centre is a proof-of-life marker for the game loop. Remove it once real gameplay exists.

## Known issues

- The wasm file is large (about 27 MB). Trunk's `wasm-opt` step failed, so it is disabled (`data-wasm-opt="0"` in `ludo-game/index.html`).
