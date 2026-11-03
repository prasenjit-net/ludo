//! Connections, the room browser and the authoritative game flow for online play.
//!
//! All state lives behind one mutex; every operation is short and synchronous.

use ludo_core::{
    Event, Game, Phase, Player,
    board::animation_ms,
    protocol::{LobbyPlayer, RoomInfo, ServerMsg},
};
use rand::Rng;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

pub type Tx = UnboundedSender<ServerMsg>;

/// A player who lets this many turns time out in a row is removed from the game.
const MAX_TIMEOUTS: u8 = 3;

/// How long an AI player "thinks" before each roll and each move, on top of the
/// time clients need to animate what just happened.
const BOT_DELAY: Duration = Duration::from_millis(400);

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
    /// An AI player: never has a connection, never times out.
    bot: bool,
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
                bot: true,
            });
        }
    }

    /// When the current turn must be played automatically: soon for an AI
    /// player, after the turn time for a person, never once the game is over.
    /// `events` are what just happened; a bot waits for them to finish animating.
    fn next_deadline(&self, turn_time: Duration, events: &[Event]) -> Option<Instant> {
        let game = self.game.as_ref().filter(|g| g.phase() != Phase::Over)?;
        let wait = Duration::from_millis(animation_ms(events))
            + if self.members[game.turn].bot {
                BOT_DELAY
            } else {
                turn_time
            };
        Some(Instant::now() + wait)
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
}

#[derive(Clone)]
pub struct Hub {
    inner: Arc<Mutex<Inner>>,
    turn_time: Duration,
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

    fn error(&self, conn: u64, message: &str) {
        self.send(
            conn,
            ServerMsg::Error {
                message: message.to_string(),
            },
        );
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
    fn push_room(&self, id: u32, events: Vec<Event>) {
        let Some(room) = self.rooms.get(&id) else {
            return;
        };
        let players = room.players();
        let bot_turn = room.game.as_ref().is_some_and(|g| room.members[g.turn].bot);
        let turn_ms = room
            .deadline
            .filter(|_| !bot_turn)
            .map(|d| d.saturating_duration_since(Instant::now()).as_millis() as u32);
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

    fn start_game(&mut self, id: u32, turn_time: Duration) {
        let Some(room) = self.rooms.get_mut(&id) else {
            return;
        };
        room.add_bots();
        // Seats follow board order so turns go clockwise.
        room.members.sort_by_key(|m| m.color.index());
        room.game = Some(Game::new(room.members.iter().map(|m| m.color).collect()));
        room.deadline = room.next_deadline(turn_time, &[]);
        self.push_room(id, Vec::new());
        self.push_rooms();
    }

    /// Take a connection out of its room. An explicit `leave` removes the player
    /// from a running game; a dropped connection only marks them away, so they
    /// can come back (the turn timer plays for them meanwhile).
    /// Returns true if the connection was in a room.
    fn detach(&mut self, conn: u64, leave: bool) -> bool {
        let Some(id) = self.conns.get_mut(&conn).and_then(|c| c.room.take()) else {
            return false;
        };
        let Some(room) = self.rooms.get_mut(&id) else {
            return true;
        };
        let Some(seat) = room.members.iter().position(|m| m.conn == Some(conn)) else {
            return true;
        };
        match &mut room.game {
            None => {
                room.members.remove(seat);
                if room.members.is_empty() {
                    self.rooms.remove(&id);
                } else {
                    if room.members.iter().all(|m| m.client_id != room.host) {
                        room.host = room.members[0].client_id.clone();
                    }
                    self.push_room(id, Vec::new());
                }
            }
            Some(game) => {
                let events = if leave { game.leave(seat) } else { Vec::new() };
                room.members[seat].conn = None;
                if room.finished() {
                    room.deadline = None;
                }
                if room.anyone_connected() {
                    self.push_room(id, events);
                } else {
                    self.rooms.remove(&id);
                }
            }
        }
        true
    }
}

impl Hub {
    pub fn new(turn_time: Duration) -> Self {
        Hub {
            inner: Arc::default(),
            turn_time,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    pub fn hello(&self, conn: u64, tx: Tx, client_id: String, name: String, color: Player) {
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

        // Reconnecting to a game in progress: take the seat back.
        let found = inner.rooms.iter().find_map(|(&id, r)| {
            let seat = r.members.iter().position(|m| m.client_id == client_id)?;
            let game = r.game.as_ref()?;
            (!game.left[seat] && !r.finished()).then_some((id, seat))
        });
        if let Some((id, seat)) = found {
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
            let room = inner.rooms.get_mut(&id).unwrap();
            room.members[seat].conn = Some(conn);
            room.members[seat].timeouts = 0;
            inner.conns.get_mut(&conn).unwrap().room = Some(id);
            inner.push_room(id, Vec::new());
        } else {
            inner.push_rooms();
        }
    }

    pub fn create_room(&self, conn: u64, name: &str, players: u8) {
        let mut inner = self.lock();
        let Some(c) = inner.conns.get(&conn) else {
            return;
        };
        if c.room.is_some() {
            return inner.error(conn, "you are already in a room");
        }
        if !(2..=4).contains(&players) {
            return inner.error(conn, "a game needs 2 to 4 players");
        }
        inner.next_room += 1;
        let id = inner.next_room;
        let c = &inner.conns[&conn];
        let member = Member {
            client_id: c.client_id.clone(),
            conn: Some(conn),
            name: c.name.clone(),
            color: c.color,
            timeouts: 0,
            bot: false,
        };
        let room = Room {
            name: clean(name, 24, &format!("{}'s room", c.name)),
            capacity: players as usize,
            host: member.client_id.clone(),
            members: vec![member],
            game: None,
            deadline: None,
        };
        inner.rooms.insert(id, room);
        inner.conns.get_mut(&conn).unwrap().room = Some(id);
        inner.push_room(id, Vec::new());
        inner.push_rooms();
    }

    pub fn join_room(&self, conn: u64, id: u32) {
        let mut inner = self.lock();
        let Some(c) = inner.conns.get(&conn) else {
            return;
        };
        if c.room.is_some() {
            return inner.error(conn, "you are already in a room");
        }
        let (client_id, name, pref) = (c.client_id.clone(), c.name.clone(), c.color);
        let Some(room) = inner.rooms.get_mut(&id) else {
            return inner.error(conn, "that room no longer exists");
        };
        if room.game.is_some() {
            return inner.error(conn, "that game has already started");
        }
        if room.members.len() >= room.capacity {
            return inner.error(conn, "that room is full");
        }
        let color = room.assign_color(pref);
        room.members.push(Member {
            client_id,
            conn: Some(conn),
            name,
            color,
            timeouts: 0,
            bot: false,
        });
        let full = room.members.len() == room.capacity;
        inner.conns.get_mut(&conn).unwrap().room = Some(id);
        if full {
            inner.start_game(id, self.turn_time);
        } else {
            inner.push_room(id, Vec::new());
            inner.push_rooms();
        }
    }

    pub fn set_color(&self, conn: u64, color: Player) {
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
            inner.push_room(id, Vec::new());
        }
    }

    pub fn start(&self, conn: u64) {
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
            return inner.error(conn, "only the host can start the game");
        }
        inner.start_game(id, self.turn_time);
    }

    pub fn leave(&self, conn: u64) {
        let mut inner = self.lock();
        if inner.detach(conn, true) {
            inner.push_rooms();
        }
    }

    pub fn disconnect(&self, conn: u64) {
        let mut inner = self.lock();
        inner.detach(conn, false);
        inner.conns.remove(&conn);
        inner.push_rooms();
    }

    /// Run a game action for `conn`, checking it is that player's turn.
    pub fn act(
        &self,
        conn: u64,
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
            return inner.error(conn, "the game has not started");
        };
        if game.turn != seat {
            return inner.error(conn, "it is not your turn");
        }
        match action(game) {
            Ok(events) => {
                room.members[seat].timeouts = 0;
                room.deadline = room.next_deadline(self.turn_time, &events);
                inner.push_room(id, events);
            }
            Err(e) => inner.error(conn, &e.to_string()),
        }
    }

    /// Play the turn of anyone who has let their time run out. Called a few times a second.
    pub fn tick(&self) {
        let mut inner = self.lock();
        let now = Instant::now();
        let due: Vec<u32> = inner
            .rooms
            .iter()
            .filter(|(_, r)| r.deadline.is_some_and(|d| d <= now))
            .map(|(&id, _)| id)
            .collect();
        let any = !due.is_empty();
        let mut rng = rand::rng();
        for id in due {
            let Some(room) = inner.rooms.get_mut(&id) else {
                continue;
            };
            let Some(game) = room.game.as_mut() else {
                continue;
            };
            let seat = game.turn;
            let mut events = Vec::new();
            let mut kicked = None;
            if room.members[seat].bot {
                // One step per tick (roll, then move) so people can follow along.
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
                let member = &mut room.members[seat];
                member.timeouts += 1;
                if member.timeouts >= MAX_TIMEOUTS && !room.finished() {
                    events.extend(room.game.as_mut().unwrap().leave(seat));
                    kicked = room.members[seat].conn.take();
                }
            }
            room.deadline = room.next_deadline(self.turn_time, &events);
            let alive = room.anyone_connected();
            if let Some(conn) = kicked {
                if let Some(c) = inner.conns.get_mut(&conn) {
                    c.room = None;
                }
                inner.send(
                    conn,
                    ServerMsg::Removed {
                        reason: "Removed for being inactive".into(),
                    },
                );
            }
            if alive {
                inner.push_room(id, events);
            } else {
                inner.rooms.remove(&id);
            }
        }
        if any {
            inner.push_rooms();
        }
    }
}
