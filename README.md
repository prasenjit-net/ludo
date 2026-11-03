# Ludo

A 2D Ludo game built with [Bevy](https://bevy.org) (0.20), compiled to WebAssembly and served by a small Rust web server. Play 2 to 4 players on the same screen, or online in a room with friends. Designed phone-first: portrait layout with big touch targets, and a side-by-side layout in landscape.

## Layout

| Crate | Description |
|---|---|
| [`ludo-core/`](ludo-core/) | Rules engine and the client/server protocol. Pure Rust, no I/O, unit tested. Shared by the other two crates. |
| [`ludo-game/`](ludo-game/) | The game: a Bevy app compiled to `wasm32-unknown-unknown` and bundled with [Trunk](https://trunkrs.dev). Everything on screen is Bevy: the board and tokens (`board.rs`, `play.rs`) and all menus and the HUD (`ui.rs`, built with `bevy_ui`, sized in `vmin` so it scales with the screen). `index.html` is just a viewport and a stylesheet. |
| [`server/`](server/) | An axum server that embeds the game build (`ludo-game/dist`) and hosts online rooms over a WebSocket at `/ws`. |

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
| `make dev` | Hot-reloading `trunk serve` on port 8080; it proxies `/ws` to a server on port 3000 (`cargo run -p server`) |
| `make test` | Unit tests for the rules engine |
| `make check` | `cargo check` the server and the wasm game |
| `make clean` | Remove build output |

The game is embedded at compile time, so the server must be rebuilt after the game changes. `make run` does this for you.

## How to play

- **Same screen:** pick 2, 3 or 4 players and take turns on one screen.
- **Online:** one player hosts a room for 2 to 4 players and shares the 4-digit code; the others tap **Join a room** and type it on the on-screen keypad. The game starts when the room is full. The server rolls the dice and validates every move, so clients cannot cheat. If a player disconnects, the server plays their turns so the game keeps going.
- Tap **ROLL**, then tap one of the highlighted tokens.

## Rules implemented

- Players sit clockwise: Red, Green, Yellow, Blue. 2 players use Red and Yellow; 3 use Red, Green and Yellow.
- A token needs a **6** to leave the yard. Moves are exact: a token cannot overshoot home.
- A **6**, a **capture** or getting a token **home** earns another roll. Three sixes in a row forfeit the turn.
- Landing on an opponent captures it (back to its yard), except on safe squares: the coloured start cells and the grey star cells.
- Tokens of the same colour may share a square; there are no blocks.
- The first player to bring all four tokens home wins and the game ends.

## Known limitations

- No reconnecting to a game in progress: a player who drops is replaced by auto-play.
- The wasm bundle is about 15 MB (5.6 MB gzipped; the server gzips responses). Release builds use fat LTO and `opt-level = "z"` to keep it small, so they take a few minutes.
- Touch and layout were tested in headless Chrome with device emulation, not on a physical phone.
- Rooms live in server memory and are lost on restart.
