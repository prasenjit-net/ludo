# Multiplayer resiliency plan

Goal: online games survive unstable networks. A player whose connection drops, for any length of time, gets their seat back when they return, and a game is never lost because of a short disconnect.

Deployment context: a single Docker container behind Cloudflare. Games are kept in memory only; saving them across server restarts is out of scope for now.

## Why a dropped player never gets back in today

Ranked by impact.

1. **Offline players are kicked as if they were idle.** `tick()` ([server/src/hub.rs](server/src/hub.rs), around line 536) counts a timeout for every human whose time runs out, without checking whether they are connected. After 3 timeouts it calls `game.leave(seat)`, and `hello()` (around line 329) then refuses to restore the seat (`!game.left[seat]`). With 10-second turns, being offline for about 30–60 seconds removes a player for good.
2. **A room is deleted the moment no human is connected** (`detach()` around line 280, `tick()` around line 551). In a solo game against bots, any network blip destroys the game. If two phone players both drop at the same time, the game is gone.
3. **Neither side checks that the connection is alive.** The server waits forever for the next message ([server/src/main.rs](server/src/main.rs), line 76) and never pings; the client never checks either. A half-open connection (phone switching networks, sleep, NAT timeout) goes unnoticed for minutes, while the server still treats the player as connected and the timer kicks them. Cloudflare also closes WebSockets after 100 seconds without traffic.
4. **The client's reconnect logic is fragile** ([ludo-game/src/net.rs](ludo-game/src/net.rs), [ludo-game/src/play.rs](ludo-game/src/play.rs)):
   - There is no time limit on opening a connection. If an attempt hangs, no close event fires, nothing schedules a retry, and the screen says "Connecting..." forever.
   - Retries are a fixed 3 seconds, with no increasing delay and no randomness.
   - It doesn't react when the network comes back or the tab becomes visible again.
   - The retry countdown only runs while the game is drawing frames, and the browser stops those when the tab is hidden.
5. **The client throws the game away when the connection drops.** `handle_net` ([ludo-game/src/session.rs](ludo-game/src/session.rs), around line 411) clears the board and goes back to the room list, and the error message is cleared when the connection reopens. If the seat was lost, the player ends up in the room list with no explanation.
6. **A drop in the waiting room removes the player from the room** (`detach()` around line 263). If they were alone, the room is deleted.
7. **Taps can be lost or land on the wrong turn.** A roll or move sent while the connection is dying is silently dropped, and nothing stops a delayed move from being applied to a newer turn.
8. **No version check.** After a deploy, a browser running the old cached game can't read the new server's messages and ignores them without any error.
9. **No server logs** for connects, disconnects or reconnects, so failures can't be diagnosed.
10. **Every restart or redeploy ends every game,** because games live only in the server's memory (accepted for now).

## Architecture

**Keep:** the server-owned game, WebSocket, and sending the full game state in each update. Because every update carries the full state, catching a reconnected player up is just "send the state again".

**Change:**

- **Separate the player from the connection (server).** Today a seat stores `conn: Option<u64>`, tying the seat to one connection. Each seat gets a presence that survives the connection: connected, offline (since a given time) or idle. Connections attach to and detach from the seat.
- **Heartbeat on both sides,** so dead connections are detected in seconds, not minutes, and Cloudflare never sees an idle socket.
- **A client connection manager** that runs on browser timers rather than the game's frame loop, retries forever with increasing delays, and keeps the board on screen while it reconnects.
- **Message format version 2,** so the client learns the outcome of a reconnect, and stale taps are rejected.

### Rules

- **Nobody is removed from a game** except by tapping **Leave game**.
- **Offline player:** each of their turns waits `LUDO_AWAY_TURN_MS` (15 s), then a bot plays it. They can take the seat back at any time until the game ends.
- **Idle player** (connected, but 3 timeouts in a row): treated like an offline player; bots play their turns with the same 15-second wait. Acting during that wait (for example tapping Roll) takes the seat back.
- **No human connected:** the game pauses (no turn timer) and the room is kept for `LUDO_ROOM_GRACE_MS` (15 min), then deleted. A returning player resumes the paused game.
- **Waiting room:** a disconnected player stays listed as away for `LUDO_LOBBY_GRACE_MS` (60 s) and keeps the host role, then is removed. While away they still hold a seat, so the room can fill and auto-start with them in it; they then rejoin the game.

### Settings (environment variables)

| Variable | Default | Meaning |
|---|---|---|
| `LUDO_TURN_MS` | 10000 | Turn time for an active player (already exists) |
| `LUDO_AWAY_TURN_MS` | 15000 | Wait before a bot plays the turn of an offline or idle player |
| `LUDO_IDLE_TIMEOUTS` | 3 | Timeouts in a row before a player counts as idle |
| `LUDO_ROOM_GRACE_MS` | 900000 | How long a room is kept, paused, when no human is connected |
| `LUDO_LOBBY_GRACE_MS` | 60000 | How long a disconnected player keeps their place in a waiting room |
| `LUDO_PING_MS` | 15000 | Heartbeat interval |
| `LUDO_DEAD_MS` | 45000 | How long without any message before a connection counts as dead |

## Phase 1: stop losing seats (mostly server)

1. **Track presence per seat** in `server/src/hub.rs`: whether the player is connected, when they went offline, and whether they are idle. The time limit for each turn depends on who plays it: 400 ms for a bot, `LUDO_TURN_MS` for an active human, `LUDO_AWAY_TURN_MS` for an offline or idle human.
2. **Rewrite the timer handling** (`tick()`):
   - An active human whose time runs out gets an automatic move as today, and their timeout count goes up. At `LUDO_IDLE_TIMEOUTS` they are marked idle instead of being removed.
   - An offline or idle human's turn is played by the bot logic in two visible steps: roll, then move.
   - Any action from the player clears the idle flag.
   - Remove the "Removed for being inactive" path.
3. **Change room lifetime:** pause the game when no human is connected, record when it emptied, and delete it from `tick()` once it has been empty longer than `LUDO_ROOM_GRACE_MS`. `detach()` no longer deletes rooms.
4. **Grace period in the waiting room** as described in the rules; `hello()` also restores players to waiting rooms.
5. **Heartbeat:**
   - The server sends a `Ping` message every `LUDO_PING_MS`; the client replies `Pong` directly from its WebSocket message handler in `net.rs`, independent of the game's frame loop.
   - The server's read loop (`server/src/main.rs`) gets a `LUDO_DEAD_MS` limit: no message at all means the connection is dead and the player is marked offline.
   - The new messages are added in a backward-compatible way: a client says in `Hello` that it answers pings, and the server does not time out one that doesn't.
6. **Logging:** add `tracing` and log connect, disconnect (with the reason), reconnect result, player marked idle and room expiry, with the client id shortened and the IP taken from `CF-Connecting-IP`.
7. **Unit tests for the hub:** `tick()` takes the current time as a parameter so the rules can be tested with a fake clock.

## Phase 2: client connection manager and reconnect experience

1. **Rewrite `ludo-game/src/net.rs` as a connection state machine** on browser timers (`setTimeout`), so it keeps working while the tab is hidden.
   - Steps: Connecting (give up after 8 s) → Open → Ready → Lost → wait and retry.
   - Retry delays of 0.5, 1, 2, 4 and 8 s, then every 10 s, each varied by ±30%. Never give up.
2. **Watchdog:** if nothing arrives from the server for 40 s, force a reconnect.
3. **Browser events:** reconnect immediately on `online` and `pageshow` (back/forward cache restore); on `visibilitychange` to visible, check that the connection is still alive.
4. **Keep the board on screen when the connection drops** (`handle_net` in `session.rs`): show a "Reconnecting…" banner over the game with input disabled, and when the game state arrives again, update the board in place instead of rebuilding it.
5. **Remember the room** id in local storage, so a page reload shows "Rejoining your game…".
6. **Show who is offline:** players see which seats are offline or idle, with the countdown before the bot plays that turn.

## Phase 3: message format, version 2 (`ludo-core/src/protocol.rs`)

1. **Reconnect outcome:** `Hello` carries a protocol version and the room id and `rev` the client last saw. The server answers with the outcome: back in your game, back in your waiting room, or not restored with a reason (game finished, room expired). The client shows the reason instead of landing silently in the room list.
2. **Reject stale taps:** every room and game update carries `rev`, a number the server raises on every change. `Roll` and `Play` include the `rev` they were made against; the server rejects stale ones with an error code and the client waits for the current state.
3. **Error codes** in addition to the error text.
4. **Version check:** if the protocol version doesn't match, the client reloads once to fetch the new game.
5. **Clean shutdown:** on SIGTERM the server tells clients it is restarting before exiting. Games are still lost on a restart, but players get a clear message instead of a hang.

## Testing (alongside every phase)

- **`server/tests/e2e.mjs` with a Node TCP proxy** in front of the server, so each connection can be cut abruptly, silently stalled (a dead connection) or delayed.
- **Scenarios:**
  - offline for more than 5 turns, then reconnect and get the seat back
  - a dead connection is detected and the player marked offline within `LUDO_DEAD_MS`
  - a solo game against bots survives a drop
  - both humans offline: the game pauses, then resumes
  - a room expires after the grace period (short setting in the test)
  - a drop in the waiting room keeps the place and host role
  - an idle player is never removed and takes the seat back by acting
  - a stale `Play` is rejected
- **`?chaos=1` in the client** drops the connection at random every 20–60 s, so whole games can be played while reconnecting is constantly exercised.
- **Run the chaos client through Cloudflare** before calling phase 2 done.

## Cloudflare and Docker checklist

- WebSockets are enabled for the zone (on by default).
- No "Cache Everything" rule caches `index.html`; otherwise browsers keep running the old game after a deploy.
- Heartbeat interval stays well under Cloudflare's 100-second idle limit.
- Expect Cloudflare to drop WebSockets occasionally during its own maintenance; phase 2's reconnect handles it.
- One container only: the game state is in memory, so a second instance would not know about games on the first.

## Later

- Save games to disk (JSON or SQLite plus a Docker volume) so restarts and deploys don't end games.
- Optionally run each room as its own task with its own timer instead of one global lock checked every 250 ms.
- Optionally replace the client-chosen `client_id` with a server-issued session token, so knowing someone's id isn't enough to take their seat.
