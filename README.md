# Ludo

A 2D Ludo game built with [Bevy](https://bevy.org) (0.20), compiled to WebAssembly and served by a small Rust web server. Play 2 to 4 players on the same screen, or online in rooms you browse, join or create. Designed phone-first: portrait layout with big touch targets, and a side-by-side layout in landscape.

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
| `make e2e` | End-to-end test of rooms, leaving, the turn timer and reconnecting (needs Node 22+) |
| `make check` | `cargo check` the server and the wasm game |
| `make clean` | Remove build output |

The game is embedded at compile time, so the server must be rebuilt after the game changes. `make run` does this for you.

## How to play

- **First visit:** pick a name and a colour (a random name is pre-filled). They are saved in your browser's local storage, and you can change them from the chip at the top right of the room list.
- **Room list:** the first screen. Each room shows its name, host, seats taken and whether it is waiting or in game. Tap **Join** on a waiting room, or **Create a room**: give it a name and the number of players it allows (2 to 4). You join your own room straight away and wait for others.
- **Waiting room:** the host can start once at least 2 players are in; the game also starts by itself when the room is full. You get your preferred colour if it is free, otherwise the first free one, and you can switch to a free colour while waiting.
- **Same screen:** under the room list, start a 2, 3 or 4 player game on one device, no server needed.
- **Playing:** tap **ROLL**, then tap one of the highlighted tokens. The server rolls the dice and validates every move.
- **Turn timer:** if you do nothing for 10 seconds, the server plays your turn (it rolls, then makes a sensible move: capture, then reach home, then leave the yard, then advance). Three timeouts in a row remove you from the game.
- **Leaving:** **Leave game** removes you and your tokens. Play goes on while at least 2 players remain; the last player left wins. If your connection drops, your seat is kept and played by the timer, and you get it back when you reconnect (same browser).

## Rules implemented

- Players sit clockwise: Red, Green, Yellow, Blue; turns follow that order for whichever colours are playing.
- A token needs a **6** to leave the yard. Moves are exact: a token cannot overshoot home.
- A **6**, a **capture** or getting a token **home** earns another roll. Three sixes in a row forfeit the turn.
- Landing on an opponent captures it (back to its yard), except on safe squares: the coloured start cells and the grey star cells.
- Tokens of the same colour may share a square; there are no blocks.
- The first player to bring all four tokens home wins and the game ends.

## Known limitations

- Reconnecting relies on the id saved in the browser, so it only works from the same browser. A room with nobody connected is deleted.
- Typing was tested with a keyboard in headless Chrome; I could not check the on-screen keyboard on a physical phone.
- The wasm bundle is about 15 MB (5.6 MB gzipped; the server gzips responses). Release builds use fat LTO and `opt-level = "z"` to keep it small, so they take a few minutes.
- Touch and layout were tested in headless Chrome with device emulation, not on a physical phone.
- Rooms live in server memory and are lost on restart.
