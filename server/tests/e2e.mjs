// End-to-end test of the room/turn flow. Run via `make e2e` (needs Node 22+).
const URL = process.env.LUDO_WS || "ws://127.0.0.1:3100/ws";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failed = 0;
const check = (c, m) => { if (!c) { failed++; console.log("FAIL:", m); } else console.log("ok:", m); };

async function client(name, color, id = name + Math.random()) {
  const ws = new WebSocket(URL); const msgs = []; let waiter = null;
  ws.onmessage = (e) => { msgs.push(JSON.parse(e.data)); waiter?.(); };
  await new Promise((r) => (ws.onopen = r));
  const c = {
    ws, msgs, id, name,
    send: (m) => ws.send(JSON.stringify(m)),
    hello: () => c.send({ type: "hello", client_id: id, name, color }),
    // wait for a message matching pred (consumes everything up to and including it)
    next: (pred, ms = 3000) => new Promise((res, rej) => {
      const t = setTimeout(() => rej(new Error("timeout waiting on " + name + " " + pred)), ms);
      const check = () => { const i = msgs.findIndex(pred); if (i >= 0) { clearTimeout(t); waiter = null; res(msgs.splice(0, i + 1).at(-1)); } else waiter = check; };
      check();
    }),
    last: (type) => msgs.filter((m) => m.type === type).at(-1),
    clear: () => (msgs.length = 0),
  };
  c.hello();
  return c;
}
const is = (type) => (m) => m.type === type;

// --- room list + create + join -------------------------------------------------
const ann = await client("Ann", "Green"), bob = await client("Bob", "Green"), cat = await client("Cat", "Blue");
let rooms = (await ann.next(is("rooms"))).rooms;
check(rooms.length === 0, "empty room list at start");
ann.send({ type: "create_room", name: "Ann's table", players: 3 });
const lobby = await ann.next(is("room"));
check(lobby.name === "Ann's table" && lobby.capacity === 3 && lobby.you === 0 && lobby.host === 0, "creator joins as host");
rooms = (await bob.next((m) => m.type === "rooms" && m.rooms.length === 1)).rooms;
check(rooms[0].joined === 1 && rooms[0].capacity === 3 && !rooms[0].playing && rooms[0].host === "Ann", "listed with seats and status");

bob.send({ type: "join_room", id: rooms[0].id });
const bl = await bob.next(is("room"));
check(bl.players.map((p) => p.player).join() === "Green,Red", "colour conflict -> first free colour");
ann.send({ type: "set_color", color: "Green" }); // taken by Ann herself -> no-op fine
bob.send({ type: "set_color", color: "Green" }); // taken -> ignored
bob.send({ type: "set_color", color: "Yellow" });
const bl2 = await bob.next((m) => m.type === "room" && m.players[1].player === "Yellow");
check(bl2.players[1].player === "Yellow", "can switch to a free colour");

// host-only start; start with 2 of 3
bob.send({ type: "start_game" });
check((await bob.next(is("error"))).message.includes("host"), "non-host cannot start");
ann.send({ type: "start_game" });
const st = await ann.next(is("state"));
check(st.game.seats.join() === "Green,Yellow" && st.you === 0 && st.turn_ms > 0, "host starts with 2 players, seats follow board order");
await bob.next(is("state"));

// started room is listed as playing and cannot be joined
rooms = (await cat.next((m) => m.type === "rooms" && m.rooms[0]?.playing)).rooms;
check(rooms[0].playing, "room shows as playing");
cat.send({ type: "join_room", id: rooms[0].id });
check((await cat.next(is("error"))).message.includes("started"), "cannot join a started game");

// --- leaving mid-game ends 2-player game ---------------------------------------
bob.send({ type: "leave_room" });
const over = await ann.next((m) => m.type === "state" && m.game.winner != null);
check(over.game.winner === 0 && over.events.some((e) => e.Left), "last player wins when the other leaves");
ann.send({ type: "leave_room" });
await bob.next((m) => m.type === "rooms" && m.rooms.length === 0);
check(true, "finished room disappears from the list");

// --- auto-start when full, leave continues with 3, timeout autoplay ------------
const [p1, p2, p3] = [await client("P1", "Red"), await client("P2", "Green"), await client("P3", "Yellow")];
p1.send({ type: "create_room", name: "Trio", players: 3 });
await p1.next(is("room"));
const id3 = (await p2.next((m) => m.type === "rooms" && m.rooms.length === 1)).rooms[0].id;
p2.send({ type: "join_room", id: id3 }); p3.send({ type: "join_room", id: id3 });
const s1 = await p1.next((m) => m.type === "state");
check(s1.game.seats.length === 3, "room auto-starts when full");

// P2 leaves; the game goes on with Red and Yellow
p2.send({ type: "leave_room" });
const afterLeave = await p1.next((m) => m.type === "state" && m.game.left[1]);
check(afterLeave.game.winner == null && afterLeave.events.some((e) => e.Left), "game continues with 2 players left");

// nobody acts: server auto-plays (turn time is short in this run)
p1.clear();
const auto = await p1.next((m) => m.type === "state" && m.events.some((e) => e.Rolled), 4000);
check(auto.events.some((e) => e.Rolled), "server auto-rolls when a player is idle");

// idle player is removed after repeated timeouts; the remaining player wins
const removed = await p1.next((m) => m.type === "removed", 15000).catch(() => null)
  ?? await p3.next((m) => m.type === "removed", 15000).catch(() => null);
check(removed && removed.reason.includes("inactive"), "idle players are removed after repeated timeouts");

// --- reconnect restores the seat -----------------------------------------------
const r1 = await client("R1", "Red", "fixed-r1"), r2 = await client("R2", "Yellow");
r1.send({ type: "create_room", name: "Re", players: 2 });
await r1.next(is("room"));
const idr = (await r2.next((m) => m.type === "rooms" && m.rooms.some((x) => x.name === "Re"))).rooms.find((x) => x.name === "Re").id;
r2.send({ type: "join_room", id: idr });
await r1.next(is("state"));
r1.ws.close();
await r2.next((m) => m.type === "state" && !m.players[0].connected);
check(true, "disconnect marks the player away");
const r1b = await client("R1", "Red", "fixed-r1");
const back = await r1b.next(is("state"));
check(back.you === 0 && back.players[0].connected, "reconnect with the same client id restores the seat");

// --- host handover in a waiting room -------------------------------------------
const h1 = await client("H1", "Red"), h2 = await client("H2", "Blue");
h1.send({ type: "create_room", name: "Handover", players: 4 });
await h1.next(is("room"));
const idh = (await h2.next((m) => m.type === "rooms" && m.rooms.some((x) => x.name === "Handover"))).rooms.find((x) => x.name === "Handover").id;
h2.send({ type: "join_room", id: idh });
await h2.next(is("room"));
h1.send({ type: "leave_room" });
const hl = await h2.next((m) => m.type === "room" && m.players.length === 1);
check(hl.host === 0 && hl.you === 0, "host leaves -> next player becomes host");

console.log(failed ? `\n${failed} FAILED` : "\nALL OK");
process.exit(failed ? 1 : 0);
