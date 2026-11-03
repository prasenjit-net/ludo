# Ludo

A 2D Ludo game for 2 to 4 players, played in the browser on phones and desktops. The game itself is a [Bevy](https://bevy.org) app compiled to WebAssembly, and it is embedded in a small Rust web server. This image is that one server binary, so there is nothing else to install or configure.

**Source and docs:** https://github.com/prasenjit-net/ludo

## Quick start

```sh
docker run --rm -p 3000:3000 {{IMAGE}}
```

Open http://localhost:3000. The first visit asks for a name and a colour, which are saved in your browser.

## Features

- **Online rooms:** browse the live room list, join one, or create your own with a name and a 2 to 4 player limit. The host can start at any time, even alone, and empty seats are filled with AI players; the game starts by itself when the room is full.
- **Same screen:** 2 to 4 players on one device, with no server needed.
- **Fair play:** the server rolls the dice and validates every move.
- **Turn timer:** if a player does nothing for 10 seconds, the server plays their turn. Players who leave are removed and the game goes on while at least 2 remain.
- **Phone-first:** portrait layout with large touch targets, and a side-by-side layout in landscape.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `LUDO_ADDR` | `0.0.0.0:3000` | Address and port the server listens on |
| `LUDO_TURN_MS` | `10000` | Milliseconds before the server plays an idle player's turn |

```sh
docker run -d --name ludo -p 8080:8080 \
  -e LUDO_ADDR=0.0.0.0:8080 -e LUDO_TURN_MS=15000 \
  {{IMAGE}}
```

## Running behind a reverse proxy

Online play uses a WebSocket at `/ws` on the same host and port as the page, so your proxy must forward WebSocket upgrades. When the page is served over HTTPS the game connects with `wss://` automatically.

## Tags

- `latest`: the newest release.
- `X.Y.Z`: an exact release, for example `0.1.1`.
- `X.Y`: the newest patch of a minor version, for example `0.1`.
- `X`: the newest release of a major version, from `1.0.0` onwards.

## Notes

- **Platform:** `linux/amd64`.
- **Image:** a distroless base running as a non-root user; there is no shell inside.
- **State:** rooms and games live in the server's memory, so a restart ends them. No volumes are needed.
- **Size:** the first page load downloads about 5.6 MB of compressed WebAssembly.

## License

Dual-licensed under either of [Apache License 2.0](https://github.com/prasenjit-net/ludo/blob/main/LICENSE-APACHE) or [MIT](https://github.com/prasenjit-net/ludo/blob/main/LICENSE-MIT), at your option. The image also contains the Fira Mono font bundled with Bevy (SIL Open Font License 1.1).
