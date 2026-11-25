// End-to-end test of the room/turn flow. Run via `make e2e` (needs Node 22+).
import net from "node:net";

const URL = process.env.LUDO_WS || "ws://127.0.0.1:3100/ws";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failed = 0;
const check = (c, m) => { if (!c) { failed++; console.log("FAIL:", m); } else console.log("ok:", m); };

async function client(name, color, id = name + Math.random(), url = URL, room = null) {
  const ws = new WebSocket(url); const msgs = []; let waiter = null;
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.type === "ping") return ws.send(JSON.stringify({ type: "pong" }));
    msgs.push(m); waiter?.();
  };
  await new Promise((r) => (ws.onopen = r));
  const c = {
    ws, msgs, id, name,
    send: (m) => ws.send(JSON.stringify(m)),
    hello: () => c.send({ type: "hello", client_id: id, name, color, heartbeat: true, room }),
    // wait for a message matching pred (consumes everything up to and including it)
    next: (pred, ms = 3000) => new Promise((res, rej) => {
      const t = setTimeout(() => rej(new Error("timeout waiting on " + name + " " + pred)), ms);
      const check = () => { const i = msgs.findIndex((m) => pred(m)); if (i >= 0) { clearTimeout(t); waiter = null; res(msgs.splice(0, i + 1).at(-1)); } else waiter = check; };
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

// host-only start; start with 2 of 3, the empty seat becomes an AI player
bob.send({ type: "start_game" });
check((await bob.next(is("error"))).message.includes("host"), "non-host cannot start");
ann.send({ type: "start_game" });
const st = await ann.next(is("state"));
check(st.game.seats.join() === "Red,Green,Yellow" && st.you === 1, "host starts with 2 of 3, a bot fills the seat in board order");
check(st.players[0].bot && st.players[0].connected && !st.players[1].bot, "the filled seat is flagged as a bot");
check(st.turn_ms == null, "no turn countdown while a bot is to play");
await bob.next(is("state"));

// started room is listed as playing and cannot be joined
rooms = (await cat.next((m) => m.type === "rooms" && m.rooms[0]?.playing)).rooms;
check(rooms[0].playing, "room shows as playing");
cat.send({ type: "join_room", id: rooms[0].id });
check((await cat.next(is("error"))).message.includes("started"), "cannot join a started game");

// --- leaving mid-game: bots keep the game going; the room goes with the last human
bob.send({ type: "leave_room" });
const after = await ann.next((m) => m.type === "state" && m.game.left[2]);
check(after.game.winner == null, "game goes on against the bot when the other human leaves");
ann.send({ type: "leave_room" });
await bob.next((m) => m.type === "rooms" && m.rooms.length === 0);
check(true, "room disappears when the last human leaves");

// --- a lone host gets bots for every other seat, and they play on their own ----
const solo = await client("Solo", "Blue");
const watcher = await client("Watcher", "Red");
solo.send({ type: "create_room", name: "Solo", players: 4 });
await solo.next(is("room"));
solo.send({ type: "start_game" });
const ss = await solo.next(is("state"));
check(ss.game.seats.length === 4 && ss.players.filter((p) => p.bot).length === 3, "host alone starts a 4-player game with 3 bots");
const rolledBy = new Set();
while (rolledBy.size < 2) {
  const m = await solo.next((m) => m.type === "state" && m.events.some((e) => e.Rolled && e.Rolled.seat !== ss.you), 10000);
  m.events.filter((e) => e.Rolled && e.Rolled.seat !== ss.you).forEach((e) => rolledBy.add(e.Rolled.seat));
}
check(rolledBy.size >= 2, "bots take their turns without anyone acting for them");
solo.send({ type: "leave_room" });
await watcher.next((m) => m.type === "rooms" && m.rooms.length === 0);
check(true, "bot-only room is removed");

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

// nobody ever acts: bots play on for them and nobody is thrown out
const idleState = await p1.next((m) => m.type === "state" && m.players[0].idle && m.players[2].idle, 30000);
check(!p1.last("removed") && !p3.last("removed"), "idle players are not removed");
check(idleState.game.left.join() === "false,true,false", "idle players keep their seats");
check(true, "and are flagged as idle");

// ...and an idle player takes the seat back just by acting
const mine = (m) => m.type === "state" && m.game.turn === m.you && m.game.dice == null && m.turn_ms != null && m.game.winner == null;
p1.clear();
await p1.next(mine, 15000);
p1.send({ type: "roll" });
const acted = await p1.next((m) => m.type === "error" || (m.type === "state" && m.events.some((e) => e.Rolled && e.Rolled.seat === m.you)), 4000);
check(acted.type === "state", "an idle player can take the seat back by rolling");
p1.send({ type: "leave_room" });
p3.send({ type: "leave_room" });

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

// a request made against an old step is refused
r1b.clear();
r1b.send({ type: "roll", rev: back.rev + 1000 });
check((await r1b.next(is("error"))).code === "stale", "an out-of-date request is rejected");

// --- being told what became of a game ------------------------------------------
r2.ws.close();
await r1b.next((m) => m.type === "state" && !m.players[1].connected);
r1b.send({ type: "leave_room" });
await sleep(100);
const late = await client("R2", "Yellow", r2.id, URL, idr);
const told = await late.next(is("welcome"));
check(told.protocol === 2 && told.resume.outcome === "finished" && told.resume.winner === "R2", "a late returner learns the game finished");
const lost = await client("Nobody", "Red", "nobody", URL, 9999);
check((await lost.next(is("welcome"))).resume.outcome === "not_found", "an unknown room is reported as not found");

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

// --- unstable networks, through a proxy that can break connections -------------
// cut(): both ends see the socket drop. stall(): the socket stays open but
// nothing gets through, like a phone that changed networks.
async function proxy() {
  const upstream = new globalThis.URL(URL);
  const links = [];
  const server = net.createServer((down) => {
    const up = net.connect(Number(upstream.port), upstream.hostname);
    const link = { down, up, stalled: false };
    links.push(link);
    down.on("data", (d) => link.stalled || up.write(d));
    up.on("data", (d) => link.stalled || down.write(d));
    for (const [a, b] of [[down, up], [up, down]]) {
      a.on("error", () => {});
      a.on("close", () => b.destroy());
    }
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return {
    url: `ws://127.0.0.1:${server.address().port}/ws`,
    cut: () => links.splice(0).forEach((l) => { l.down.destroy(); l.up.destroy(); }),
    stall: () => links.forEach((l) => (l.stalled = true)),
    close: () => { links.forEach((l) => { l.down.destroy(); l.up.destroy(); }); server.close(); },
  };
}
// Wait for a room of this name in the browser list and return its id.
const roomId = async (c, name) =>
  (await c.next((m) => m.type === "rooms" && m.rooms.some((x) => x.name === name))).rooms.find((x) => x.name === name).id;
async function duel(name, urlA, urlB) {
  const a = await client(name + "A", "Red", name + "-a", urlA);
  const b = await client(name + "B", "Yellow", name + "-b", urlB);
  a.send({ type: "create_room", name, players: 2 });
  await a.next(is("room"));
  b.send({ type: "join_room", id: await roomId(b, name) });
  const st = await a.next(is("state"));
  await b.next(is("state"));
  return { a, b, seatA: st.you, seatB: 1 - st.you };
}

const net1 = await proxy(), net2 = await proxy();

// a long absence does not cost the seat
{
  const { a, seatB } = await duel("Drop", URL, net1.url);
  net1.cut();
  await a.next((m) => m.type === "state" && !m.players[seatB].connected);
  // keep A's turns moving until B's seat has been played five times
  const playOn = async (played) => {
    if (played >= 5) return;
    const m = await a.next((m) => m.type === "state", 8000);
    if (m.game.turn === m.you && m.game.dice == null && m.turn_ms != null) a.send({ type: "roll" });
    else if (m.game.turn === m.you && m.game.dice != null) {
      // any token will do; try them until one is legal
      for (let token = 0; token < 4; token++) a.send({ type: "play", token });
    }
    return playOn(played + m.events.filter((e) => e.Rolled?.seat === seatB).length);
  };
  await playOn(0);
  const again = await client("DropB", "Yellow", "Drop-b", net1.url);
  const st = await again.next(is("state"));
  check(st.you === seatB && !st.game.left[seatB], "back after 5 or more missed turns, same seat");
  a.send({ type: "leave_room" });
  again.send({ type: "leave_room" });
}

// a silently dead connection is noticed
{
  const { a, seatB } = await duel("Stall", URL, net1.url);
  const from = Date.now();
  net1.stall();
  await a.next((m) => m.type === "state" && !m.players[seatB].connected, 5000);
  check(Date.now() - from < 3000, "a dead connection is detected by the heartbeat");
  net1.cut();
  a.send({ type: "leave_room" });
}

// a solo game against bots survives a drop
{
  const solo = await client("Lone", "Red", "lone", net1.url);
  solo.send({ type: "create_room", name: "Lone", players: 4 });
  await solo.next(is("room"));
  solo.send({ type: "start_game" });
  const before = await solo.next(is("state"));
  net1.cut();
  await sleep(800);
  const again = await client("Lone", "Red", "lone", net1.url);
  const st = await again.next(is("state"));
  check(st.you === before.you && st.game.seats.length === 4, "solo game is still there after a drop");
  again.send({ type: "leave_room" });
}

// both players drop: the game waits, then carries on
{
  const { seatA } = await duel("Both", net1.url, net2.url);
  net1.cut(); net2.cut();
  await sleep(1000);
  const watcher2 = await client("W2", "Blue");
  check((await watcher2.next(is("rooms"))).rooms.some((r) => r.name === "Both"), "game is kept when everyone drops");
  const back = await client("BothA", "Red", "Both-a", net1.url);
  const st = await back.next(is("state"));
  check(st.you === seatA && st.game.winner == null, "and resumes for whoever returns");
  back.send({ type: "leave_room" });
  watcher2.ws.close();
}

// an abandoned game expires
{
  await duel("Gone", net1.url, net2.url);
  const watcher3 = await client("W3", "Blue");
  const goneId = await roomId(watcher3, "Gone");
  net1.cut(); net2.cut();
  await watcher3.next((m) => m.type === "rooms" && !m.rooms.some((r) => r.name === "Gone"), 8000);
  check(true, "a game nobody returns to is removed");
  const tooLate = await client("GoneA", "Red", "Gone-a", URL, goneId);
  check((await tooLate.next(is("welcome"))).resume.outcome === "expired", "and its player is told it expired");
  tooLate.ws.close();
  watcher3.ws.close();
}

// a drop in the waiting room keeps the place and the host role
{
  const host = await client("WH", "Red", "wh", net1.url);
  const guest = await client("WG", "Blue", "wg");
  host.send({ type: "create_room", name: "Wait", players: 4 });
  await host.next(is("room"));
  guest.send({ type: "join_room", id: await roomId(guest, "Wait") });
  await guest.next(is("room"));
  net1.cut();
  const away = await guest.next((m) => m.type === "room" && !m.players[0].connected);
  check(away.players.length === 2 && away.host === 0, "dropped host stays listed in the waiting room");
  const again = await client("WH", "Red", "wh", net1.url);
  const room = await again.next(is("room"));
  check(room.you === 0 && room.host === 0, "and returns as host");
  net1.cut();
  const gone = await guest.next((m) => m.type === "room" && m.players.length === 1, 8000);
  check(gone.host === 0 && gone.you === 0, "after the grace period the place is given up");
  guest.send({ type: "leave_room" });
}

// --- shutting down -------------------------------------------------------------
if (process.env.LUDO_PID) {
  const last = await client("Last", "Red");
  await last.next(is("rooms"));
  process.kill(Number(process.env.LUDO_PID), "SIGTERM");
  await last.next(is("restarting"));
  check(true, "clients are warned before the server stops");
}

net1.close();
net2.close();

console.log(failed ? `\n${failed} FAILED` : "\nALL OK");
process.exit(failed ? 1 : 0);
