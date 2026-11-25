//! Connections, the room browser and the authoritative game flow for online play.
//!
//! All state lives behind one mutex; every operation is short and synchronous.

use ludo_core::{
    Event, Game, Phase, Player,
    board::animation_ms,
    protocol::{ErrorCode, LobbyPlayer, PROTOCOL, Resumed, RoomInfo, ServerMsg},
};
use rand::Rng;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;
use tracing::info;

pub type Tx = UnboundedSender<ServerMsg>;

/// How long the server waits for things.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// Turn time of a player who is there and playing.
    pub turn: Duration,
    /// Wait before a bot plays the turn of an offline or idle player.
    pub away_turn: Duration,
    /// Timeouts in a row before a connected player counts as idle.
    pub idle_timeouts: u8,
    /// How long a game with nobody connected is kept, paused.
    pub room_grace: Duration,
    /// How long a disconnected player keeps their place in a waiting room.
    pub lobby_grace: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            turn: Duration::from_secs(10),
            away_turn: Duration::from_secs(15),
            idle_timeouts: 3,
            room_grace: Duration::from_secs(15 * 60),
            lobby_grace: Duration::from_secs(60),
        }
    }
}

/// How long an AI player "thinks" before each roll and each move, on top of the
/// time clients need to animate what just happened.
const BOT_DELAY: Duration = Duration::from_millis(400);

/// A player who has just come back gets at least this long to act (or the whole
/// turn time, if that is shorter).
const MIN_RETURN: Duration = Duration::from_secs(3);

/// How many closed games are remembered, to tell late returners what happened.
const REMEMBERED: usize = 256;

/// Who a client says it is, on connecting.
pub struct Hello {
    pub client_id: String,
    pub name: String,
    pub color: Player,
    /// The room it believes it is in.
    pub room: Option<u32>,
}

/// A game that has been deleted.
struct Ended {
    room: u32,
    clients: Vec<String>,
    outcome: Resumed,
}

/// Client ids give access to a seat, so logs only show the start of one.
fn short(client_id: &str) -> String {
    client_id.chars().take(8).collect()
}

struct Conn {
    tx: Tx,
    client_id: String,
    name: String,
    color: Player,
    room: Option<u32>,
}

struct Member {
    client_id: String,
    /// `None` while the player is away (disconnected).
    conn: Option<u64>,
    name: String,
    color: Player,
    timeouts: u8,
    /// Connected but not playing: turns are played for them until they act.
    idle: bool,
    /// When the connection dropped.
    away_since: Option<Instant>,
    /// An AI player: never has a connection, never times out.
    bot: bool,
}

impl Member {
    fn person(client_id: String, conn: u64, name: String, color: Player) -> Self {
        Member {
            client_id,
            conn: Some(conn),
            name,
            color,
            timeouts: 0,
            idle: false,
            away_since: None,
            bot: false,
        }
    }

    /// How long this player's turn lasts before the server steps in.
    fn turn_wait(&self, timing: Timing) -> Duration {
        if self.bot {
            BOT_DELAY
        } else if self.conn.is_none() || self.idle {
            timing.away_turn
        } else {
            timing.turn
        }
    }
}

struct Room {
    name: String,
    capacity: usize,
    /// Members are in seat order once the game has started.
    members: Vec<Member>,
    /// `client_id` of the host.
    host: String,
    game: Option<Game>,
    deadline: Option<Instant>,
    /// Losing or regaining a connection never moves `deadline` far past this,
    /// so a flapping connection cannot stall the game.
    latest: Option<Instant>,
    /// The server is playing the current turn for an offline or idle player.
    autopilot: bool,
    /// Since when no person has been connected to this game (it is paused).
    empty_since: Option<Instant>,
    /// Counts the steps of the game, to recognise requests that are too late.
    rev: u64,
}

impl Room {
    fn host_index(&self) -> usize {
        self.members
            .iter()
            .position(|m| m.client_id == self.host)
            .unwrap_or(0)
    }

    fn players(&self) -> Vec<LobbyPlayer> {
        self.members
            .iter()
            .map(|m| LobbyPlayer {
                name: m.name.clone(),
                player: m.color,
                connected: m.bot || m.conn.is_some(),
                idle: m.idle,
                bot: m.bot,
            })
            .collect()
    }

    fn finished(&self) -> bool {
        self.game.as_ref().is_some_and(|g| g.phase() == Phase::Over)
    }

    /// Is a person still attached? Bots alone do not keep a room alive.
    fn anyone_connected(&self) -> bool {
        self.members.iter().any(|m| m.conn.is_some())
    }

    /// Fill every empty seat with an AI player, in the first free colours.
    fn add_bots(&mut self) {
        let mut n = 0;
        while self.members.len() < self.capacity {
            let Some(color) = Player::ALL.into_iter().find(|&c| self.color_free(c)) else {
                break;
            };
            n += 1;
            self.members.push(Member {
                client_id: format!("bot-{n}"),
                conn: None,
                name: format!("Bot {n}"),
                color,
                timeouts: 0,
                idle: false,
                away_since: None,
                bot: true,
            });
        }
    }

    /// Set the time of the next automatic step: soon for an AI player or a turn
    /// already being played for someone, otherwise when the player's wait runs
    /// out. `events` are what just happened; the wait starts once they have
    /// finished animating. Nothing is scheduled while the game is over or paused.
    fn schedule(&mut self, timing: Timing, now: Instant, events: &[Event]) {
        if !events.is_empty() {
            self.rev += 1;
        }
        let game = self.game.as_ref().filter(|g| g.phase() != Phase::Over);
        let Some(game) = game.filter(|_| self.empty_since.is_none()) else {
            self.deadline = None;
            self.latest = None;
            self.autopilot = false;
            return;
        };
        let start = now + Duration::from_millis(animation_ms(events));
        let wait = if self.autopilot {
            BOT_DELAY
        } else {
            self.members[game.turn].turn_wait(timing)
        };
        self.deadline = Some(start + wait);
        self.latest = Some(start + timing.away_turn + timing.turn);
    }

    /// The player in `seat` dropped or came back: if it is their turn, change
    /// how long is left of it.
    fn presence_changed(&mut self, seat: usize, timing: Timing, now: Instant) {
        if self.game.as_ref().is_none_or(|g| g.turn != seat) {
            return;
        }
        let member = &self.members[seat];
        if member.conn.is_some() {
            self.autopilot = false;
        } else if self.autopilot {
            return;
        }
        if let (Some(deadline), Some(latest)) = (self.deadline.as_mut(), self.latest) {
            // The floor is capped too, or rapid flapping would keep moving it.
            let grace = MIN_RETURN.min(timing.turn);
            let soonest = (now + grace).min(latest + grace);
            *deadline = (now + member.turn_wait(timing)).min(latest).max(soonest);
        }
    }

    /// Stop the clock until somebody comes back.
    fn pause(&mut self, now: Instant) {
        self.empty_since = Some(now);
        self.deadline = None;
        self.latest = None;
        self.autopilot = false;
    }

    /// Could somebody still come back to this game?
    fn resumable(&self) -> bool {
        self.game.as_ref().is_some_and(|g| {
            g.phase() != Phase::Over
                && self
                    .members
                    .iter()
                    .enumerate()
                    .any(|(seat, m)| !m.bot && !g.left[seat])
        })
    }

    /// Hand the waiting room to someone else if the host is gone.
    fn fix_host(&mut self) {
        if self.members.iter().any(|m| m.client_id == self.host) {
            return;
        }
        let next = self.members.iter().find(|m| m.conn.is_some());
        if let Some(m) = next.or(self.members.first()) {
            self.host = m.client_id.clone();
        }
    }

    fn winner_name(&self) -> Option<String> {
        let seat = self.game.as_ref()?.winner?;
        Some(self.members[seat].name.clone())
    }

    fn ended(&self, id: u32, outcome: Resumed) -> Ended {
        Ended {
            room: id,
            clients: self
                .members
                .iter()
                .filter(|m| !m.bot)
                .map(|m| m.client_id.clone())
                .collect(),
            outcome,
        }
    }

    fn color_free(&self, color: Player) -> bool {
        self.members.iter().all(|m| m.color != color)
    }

    /// The preferred colour if free, otherwise the first free one.
    fn assign_color(&self, preferred: Player) -> Player {
        if self.color_free(preferred) {
            preferred
        } else {
            Player::ALL
                .into_iter()
                .find(|&c| self.color_free(c))
                .unwrap_or(preferred)
        }
    }
}

#[derive(Default)]
struct Inner {
    conns: HashMap<u64, Conn>,
    rooms: BTreeMap<u32, Room>,
    next_room: u32,
    ended: VecDeque<Ended>,
}

#[derive(Clone)]
pub struct Hub {
    inner: Arc<Mutex<Inner>>,
    timing: Timing,
}

fn clean(text: &str, max: usize, fallback: &str) -> String {
    let text: String = text.trim().chars().take(max).collect();
    if text.is_empty() {
        fallback.to_string()
    } else {
        text
    }
}

impl Inner {
    fn send(&self, conn: u64, msg: ServerMsg) {
        if let Some(c) = self.conns.get(&conn) {
            let _ = c.tx.send(msg);
        }
    }

    fn error(&self, conn: u64, code: ErrorCode, message: &str) {
        self.send(
            conn,
            ServerMsg::Error {
                code,
                message: message.to_string(),
            },
        );
    }

    fn remember(&mut self, ended: Ended) {
        if self.ended.len() == REMEMBERED {
            self.ended.pop_front();
        }
        self.ended.push_back(ended);
    }

    /// Why this client has no seat in the room it thought it was in.
    fn fate(&self, id: u32, client_id: &str) -> Resumed {
        if let Some(room) = self.rooms.get(&id)
            && let Some(game) = &room.game
            && let Some(seat) = room.members.iter().position(|m| m.client_id == client_id)
        {
            if game.left[seat] {
                return Resumed::Left;
            }
            if let Some(winner) = room.winner_name() {
                return Resumed::Finished { winner };
            }
        }
        self.ended
            .iter()
            .rev()
            .find(|e| e.room == id && e.clients.iter().any(|c| c == client_id))
            .map_or(Resumed::NotFound, |e| e.outcome.clone())
    }

    /// Send the room browser to everyone who is not in a room.
    fn push_rooms(&self) {
        let rooms: Vec<RoomInfo> = self
            .rooms
            .iter()
            .filter(|(_, r)| !r.finished())
            .map(|(&id, r)| RoomInfo {
                id,
                name: r.name.clone(),
                host: r
                    .members
                    .iter()
                    .find(|m| m.client_id == r.host)
                    .map_or_else(String::new, |m| m.name.clone()),
                capacity: r.capacity,
                joined: r.members.len(),
                playing: r.game.is_some(),
            })
            .collect();
        for c in self.conns.values().filter(|c| c.room.is_none()) {
            let _ = c.tx.send(ServerMsg::Rooms {
                rooms: rooms.clone(),
            });
        }
    }

    /// Send a room's current view (waiting room or game) to its connected members.
    fn push_room(&self, id: u32, events: Vec<Event>, now: Instant) {
        let Some(room) = self.rooms.get(&id) else {
            return;
        };
        let players = room.players();
        // No countdown while the server is the one playing.
        let automatic =
            room.autopilot || room.game.as_ref().is_some_and(|g| room.members[g.turn].bot);
        let turn_ms = room
            .deadline
            .filter(|_| !automatic)
            .map(|d| d.saturating_duration_since(now).as_millis() as u32);
        for (you, member) in room.members.iter().enumerate() {
            let Some(conn) = member.conn else { continue };
            let msg = match &room.game {
                Some(game) => ServerMsg::State {
                    id,
                    you,
                    players: players.clone(),
                    game: game.clone(),
                    events: events.clone(),
                    turn_ms,
                    rev: room.rev,
                },
                None => ServerMsg::Room {
                    id,
                    name: room.name.clone(),
                    you,
                    host: room.host_index(),
                    capacity: room.capacity,
                    players: players.clone(),
                },
            };
            self.send(conn, msg);
        }
    }

    fn start_game(&mut self, id: u32, timing: Timing, now: Instant) {
        let Some(room) = self.rooms.get_mut(&id) else {
            return;
        };
        room.add_bots();
        // Seats follow board order so turns go clockwise.
        room.members.sort_by_key(|m| m.color.index());
        room.game = Some(Game::new(room.members.iter().map(|m| m.color).collect()));
        room.schedule(timing, now, &[]);
        self.push_room(id, Vec::new(), now);
        self.push_rooms();
    }

    /// Take a connection out of its room. An explicit `leave` gives up the
    /// seat; a dropped connection only marks the player away, so they can come
    /// back. Returns true if the connection was in a room.
    fn detach(&mut self, conn: u64, leave: bool, timing: Timing, now: Instant) -> bool {
        let Some(id) = self.conns.get_mut(&conn).and_then(|c| c.room.take()) else {
            return false;
        };
        let Some(room) = self.rooms.get_mut(&id) else {
            return true;
        };
        let Some(seat) = room.members.iter().position(|m| m.conn == Some(conn)) else {
            return true;
        };
        room.members[seat].conn = None;
        room.members[seat].away_since = Some(now);
        let Some(game) = room.game.as_mut() else {
            if leave {
                room.members.remove(seat);
            }
            if room.members.is_empty() {
                self.rooms.remove(&id);
            } else {
                room.fix_host();
                self.push_room(id, Vec::new(), now);
            }
            return true;
        };

        let turn = game.turn;
        let events = if leave { game.leave(seat) } else { Vec::new() };
        let turn_passed = game.turn != turn || game.phase() == Phase::Over;
        if turn_passed {
            room.autopilot = false;
            room.schedule(timing, now, &events);
        }
        if room.anyone_connected() {
            if !leave {
                room.presence_changed(seat, timing, now);
            }
            self.push_room(id, events, now);
        } else if room.resumable() {
            info!(room = id, "nobody connected, game paused");
            room.pause(now);
        } else {
            if let Some(winner) = room.winner_name() {
                let ended = room.ended(id, Resumed::Finished { winner });
                self.remember(ended);
            }
            self.rooms.remove(&id);
        }
        true
    }

    /// Delete games nobody came back to, and drop people who have been gone
    /// from a waiting room too long. Returns true if anything changed.
    fn expire(&mut self, timing: Timing, now: Instant) -> bool {
        let mut changed = false;
        let mut lobbies = Vec::new();
        let mut ended = Vec::new();
        self.rooms.retain(|&id, room| {
            if room.game.is_some() {
                let expired = room
                    .empty_since
                    .is_some_and(|t| now.duration_since(t) >= timing.room_grace);
                if expired {
                    info!(room = id, "nobody came back, room expired");
                    ended.push(room.ended(id, Resumed::Expired));
                    changed = true;
                }
                return !expired;
            }
            let before = room.members.len();
            room.members.retain(|m| {
                let gone = m.conn.is_none()
                    && m.away_since
                        .is_some_and(|t| now.duration_since(t) >= timing.lobby_grace);
                if gone {
                    info!(room = id, client = %short(&m.client_id), "removed from waiting room");
                }
                !gone
            });
            if room.members.len() != before {
                changed = true;
                room.fix_host();
                lobbies.push(id);
            }
            !room.members.is_empty()
        });
        for game in ended {
            self.remember(game);
        }
        for id in lobbies {
            self.push_room(id, Vec::new(), now);
        }
        changed
    }
}

impl Hub {
    pub fn new(timing: Timing) -> Self {
        Hub {
            inner: Arc::default(),
            timing,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    pub fn hello(&self, conn: u64, tx: Tx, hello: Hello, now: Instant) {
        let Hello {
            client_id,
            name,
            color,
            room: claimed,
        } = hello;
        let mut inner = self.lock();
        let name = clean(&name, 16, "Player");
        if let Some(c) = inner.conns.get_mut(&conn) {
            c.name = name;
            c.color = color;
            if c.room.is_none() {
                inner.push_rooms();
            }
            return;
        }
        inner.conns.insert(
            conn,
            Conn {
                tx,
                client_id: client_id.clone(),
                name,
                color,
                room: None,
            },
        );

        // Reconnecting: take the seat (or the place in a waiting room) back.
        let found = inner.rooms.iter().find_map(|(&id, r)| {
            let seat = r
                .members
                .iter()
                .position(|m| !m.bot && m.client_id == client_id)?;
            let open = r
                .game
                .as_ref()
                .is_none_or(|g| !g.left[seat] && !r.finished());
            open.then_some((id, seat))
        });
        let resume = match found {
            Some((id, _)) if inner.rooms[&id].game.is_some() => Some(Resumed::Game),
            Some(_) => Some(Resumed::Lobby),
            None => claimed.map(|id| inner.fate(id, &client_id)),
        };
        inner.send(
            conn,
            ServerMsg::Welcome {
                protocol: PROTOCOL,
                resume,
            },
        );
        let Some((id, seat)) = found else {
            info!(conn, client = %short(&client_id), "hello, no seat to restore");
            inner.push_rooms();
            return;
        };
        info!(conn, client = %short(&client_id), room = id, seat, "seat restored");
        let old = inner.rooms[&id].members[seat].conn;
        if let Some(old) = old
            && let Some(c) = inner.conns.get_mut(&old)
        {
            c.room = None; // a second tab took over
            inner.send(
                old,
                ServerMsg::Removed {
                    reason: "Opened in another tab".into(),
                },
            );
        }
        let timing = self.timing;
        let room = inner.rooms.get_mut(&id).unwrap();
        let member = &mut room.members[seat];
        member.conn = Some(conn);
        member.timeouts = 0;
        member.idle = false;
        member.away_since = None;
        if room.empty_since.take().is_some() && room.game.is_some() {
            room.schedule(timing, now, &[]);
        } else {
            room.presence_changed(seat, timing, now);
        }
        inner.conns.get_mut(&conn).unwrap().room = Some(id);
        inner.push_room(id, Vec::new(), now);
    }

    pub fn create_room(&self, conn: u64, name: &str, players: u8, now: Instant) {
        let mut inner = self.lock();
        let Some(c) = inner.conns.get(&conn) else {
            return;
        };
        if c.room.is_some() {
            return inner.error(conn, ErrorCode::AlreadyInRoom, "you are already in a room");
        }
        if !(2..=4).contains(&players) {
            return inner.error(
                conn,
                ErrorCode::BadPlayerCount,
                "a game needs 2 to 4 players",
            );
        }
        inner.next_room += 1;
        let id = inner.next_room;
        let c = &inner.conns[&conn];
        let member = Member::person(c.client_id.clone(), conn, c.name.clone(), c.color);
        let room = Room {
            name: clean(name, 24, &format!("{}'s room", c.name)),
            capacity: players as usize,
            host: member.client_id.clone(),
            members: vec![member],
            game: None,
            deadline: None,
            latest: None,
            autopilot: false,
            empty_since: None,
            rev: 0,
        };
        inner.rooms.insert(id, room);
        inner.conns.get_mut(&conn).unwrap().room = Some(id);
        inner.push_room(id, Vec::new(), now);
        inner.push_rooms();
    }

    pub fn join_room(&self, conn: u64, id: u32, now: Instant) {
        let mut inner = self.lock();
        let Some(c) = inner.conns.get(&conn) else {
            return;
        };
        if c.room.is_some() {
            return inner.error(conn, ErrorCode::AlreadyInRoom, "you are already in a room");
        }
        let (client_id, name, pref) = (c.client_id.clone(), c.name.clone(), c.color);
        let Some(room) = inner.rooms.get_mut(&id) else {
            return inner.error(conn, ErrorCode::RoomGone, "that room no longer exists");
        };
        if room.game.is_some() {
            return inner.error(
                conn,
                ErrorCode::GameStarted,
                "that game has already started",
            );
        }
        if room.members.len() >= room.capacity {
            return inner.error(conn, ErrorCode::RoomFull, "that room is full");
        }
        let color = room.assign_color(pref);
        room.members
            .push(Member::person(client_id, conn, name, color));
        let full = room.members.len() == room.capacity;
        inner.conns.get_mut(&conn).unwrap().room = Some(id);
        if full {
            inner.start_game(id, self.timing, now);
        } else {
            inner.push_room(id, Vec::new(), now);
            inner.push_rooms();
        }
    }

    pub fn set_color(&self, conn: u64, color: Player, now: Instant) {
        let mut inner = self.lock();
        let Some(id) = inner.conns.get(&conn).and_then(|c| c.room) else {
            return;
        };
        let Some(room) = inner.rooms.get_mut(&id) else {
            return;
        };
        if room.game.is_some() || !room.color_free(color) {
            return;
        }
        if let Some(m) = room.members.iter_mut().find(|m| m.conn == Some(conn)) {
            m.color = color;
            inner.push_room(id, Vec::new(), now);
        }
    }

    pub fn start(&self, conn: u64, now: Instant) {
        let mut inner = self.lock();
        let Some(id) = inner.conns.get(&conn).and_then(|c| c.room) else {
            return;
        };
        let Some(room) = inner.rooms.get(&id) else {
            return;
        };
        if room.game.is_some() {
            return;
        }
        if room.members[room.host_index()].conn != Some(conn) {
            return inner.error(conn, ErrorCode::NotHost, "only the host can start the game");
        }
        inner.start_game(id, self.timing, now);
    }

    pub fn leave(&self, conn: u64, now: Instant) {
        let mut inner = self.lock();
        if inner.detach(conn, true, self.timing, now) {
            inner.push_rooms();
        }
    }

    pub fn disconnect(&self, conn: u64, now: Instant) {
        let mut inner = self.lock();
        inner.detach(conn, false, self.timing, now);
        inner.conns.remove(&conn);
        inner.push_rooms();
    }

    /// Tell everyone the server is about to stop.
    pub fn announce_restart(&self) {
        let inner = self.lock();
        for c in inner.conns.values() {
            let _ = c.tx.send(ServerMsg::Restarting);
        }
    }

    /// Run a game action for `conn`, checking it is that player's turn and
    /// that the request (made against step `rev`) is not out of date.
    pub fn act(
        &self,
        conn: u64,
        rev: Option<u64>,
        now: Instant,
        action: impl FnOnce(&mut Game) -> Result<Vec<Event>, ludo_core::Error>,
    ) {
        let mut inner = self.lock();
        let Some(id) = inner.conns.get(&conn).and_then(|c| c.room) else {
            return;
        };
        let Some(room) = inner.rooms.get_mut(&id) else {
            return;
        };
        let (Some(seat), Some(game)) = (
            room.members.iter().position(|m| m.conn == Some(conn)),
            room.game.as_mut(),
        ) else {
            return inner.error(conn, ErrorCode::NotStarted, "the game has not started");
        };
        if rev.is_some_and(|r| r != room.rev) {
            return inner.error(conn, ErrorCode::Stale, "the game has moved on");
        }
        if game.turn != seat {
            return inner.error(conn, ErrorCode::NotYourTurn, "it is not your turn");
        }
        match action(game) {
            Ok(events) => {
                // Acting takes the seat back from the autopilot.
                room.members[seat].timeouts = 0;
                room.members[seat].idle = false;
                room.autopilot = false;
                room.schedule(self.timing, now, &events);
                inner.push_room(id, events, now);
            }
            Err(e) => inner.error(conn, ErrorCode::NotAllowed, &e.to_string()),
        }
    }

    /// Play the turn of anyone whose time has run out, and expire what has been
    /// abandoned. Called a few times a second.
    pub fn tick(&self, now: Instant) {
        let mut inner = self.lock();
        let timing = self.timing;
        let mut changed = inner.expire(timing, now);
        let due: Vec<u32> = inner
            .rooms
            .iter()
            .filter(|(_, r)| r.deadline.is_some_and(|d| d <= now))
            .map(|(&id, _)| id)
            .collect();
        changed |= !due.is_empty();
        let mut rng = rand::rng();
        for id in due {
            let Some(room) = inner.rooms.get_mut(&id) else {
                continue;
            };
            let Some(game) = room.game.as_mut() else {
                continue;
            };
            let seat = game.turn;
            let member = &mut room.members[seat];
            let mut events = Vec::new();
            if member.bot || member.idle || member.conn.is_none() {
                // The server plays: one step per tick (roll, then move) so
                // people can follow along.
                room.autopilot = !member.bot;
                if game.dice.is_none() {
                    events.extend(game.roll(rng.random_range(1..=6)).unwrap_or_default());
                } else if let Some(token) = game.best_move() {
                    events.extend(game.play(token).unwrap_or_default());
                }
            } else {
                if game.dice.is_none() {
                    events.extend(game.roll(rng.random_range(1..=6)).unwrap_or_default());
                }
                if let Some(token) = game.auto_move() {
                    events.extend(game.play(token).unwrap_or_default());
                }
                member.timeouts = member.timeouts.saturating_add(1);
                if member.timeouts >= timing.idle_timeouts {
                    member.idle = true;
                    info!(room = id, seat, client = %short(&member.client_id), "player is idle");
                }
            }
            if game.turn != seat {
                room.autopilot = false;
            }
            room.schedule(timing, now, &events);
            inner.push_room(id, events, now);
        }
        if changed {
            inner.push_rooms();
        }
    }
}

#[cfg(test)]
mod tests;
