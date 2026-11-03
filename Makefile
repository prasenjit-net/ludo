.PHONY: all game server run dev check clean

all: server

# Build the wasm game with trunk (output: ludo-game/dist).
game:
	cd ludo-game && trunk build --release

# Build the server, which embeds ludo-game/dist at compile time.
server: game
	cargo build --release -p server

# Build everything and serve at http://127.0.0.1:3000 (override with LUDO_ADDR).
run: game
	cargo run --release -p server

# Hot-reloading dev server for the game only (trunk, port 8080).
dev:
	cd ludo-game && trunk serve

check:
	cargo check -p server
	cargo check -p ludo-game --target wasm32-unknown-unknown

clean:
	cargo clean
	rm -rf ludo-game/dist
