//! Room registry and the authoritative game flow for online play.

use ludo_core::{
    Event, Game, Phase, Player,
    protocol::{LobbyPlayer, ServerMsg},
};
use rand::Rng;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::UnboundedSender;

pub type Tx = UnboundedSender<ServerMsg>;

struct Member {
    conn: u64,
    name: String,
    tx: Tx,
    connected: bool,
}

struct Room {
    capacity: usize,
    members: Vec<Member>,
    game: Option<Game>,
}

impl Room {
    fn seat_of(&self, conn: u64) -> Option<usize> {
        self.members.iter().position(|m| m.conn == conn)
    }

    fn lobby_players(&self) -> Vec<LobbyPlayer> {
        let colors = Player::for_count(self.capacity);
        self.members
            .iter()
            .enumerate()
            .map(|(i, m)| LobbyPlayer {
                name: m.name.clone(),
                player: colors[i],
                connected: m.connected,
            })
            .collect()
    }

    fn broadcast_lobby(&self, code: &str) {
        let players = self.lobby_players();
        for (you, m) in self.members.iter().enumerate() {
            let _ = m.tx.send(ServerMsg::Lobby {
                code: code.to_string(),
                you,
                capacity: self.capacity,
                players: players.clone(),
            });
        }
    }

    fn broadcast_state(&self, code: &str, events: Vec<Event>) {
        let Some(game) = &self.game else { return };
        let players = self.lobby_players();
        for (you, m) in self.members.iter().enumerate() {
            let _ = m.tx.send(ServerMsg::State {
                code: code.to_string(),
                you,
                players: players.clone(),
                game: game.clone(),
                events: events.clone(),
            });
        }
    }

    /// Play the turns of disconnected players so the game never stalls.
    fn autoplay(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        let Some(game) = &mut self.game else { return events };
        let mut rng = rand::rng();
        while game.phase() != Phase::Over && !self.members[game.turn].connected {
            events.extend(game.roll(rng.random_range(1..=6)).unwrap_or_default());
            if let Some(&token) = game.movable().first() {
                events.extend(game.play(token).unwrap_or_default());
            }
        }
        events
    }
}

#[derive(Clone, Default)]
pub struct Rooms {
    inner: Arc<Mutex<HashMap<String, Room>>>,
}

fn error(tx: &Tx, message: impl Into<String>) {
    let _ = tx.send(ServerMsg::Error { message: message.into() });
}

fn clean_name(name: &str, fallback: usize) -> String {
    let name: String = name.trim().chars().take(16).collect();
    if name.is_empty() { format!("Player {fallback}") } else { name }
}

fn new_code(rooms: &HashMap<String, Room>) -> String {
    let mut rng = rand::rng();
    loop {
        let code: String = (0..4).map(|_| rng.random_range(b'A'..=b'Z') as char).collect();
        if !rooms.contains_key(&code) {
            return code;
        }
    }
}

impl Rooms {
    pub fn create(&self, conn: u64, tx: Tx, name: &str, players: u8) -> Option<String> {
        if !(2..=4).contains(&players) {
            error(&tx, "a game needs 2 to 4 players");
            return None;
        }
        let mut rooms = self.inner.lock().unwrap();
        let code = new_code(&rooms);
        let room = Room {
            capacity: players as usize,
            members: vec![Member { conn, name: clean_name(name, 1), tx, connected: true }],
            game: None,
        };
        room.broadcast_lobby(&code);
        rooms.insert(code.clone(), room);
        Some(code)
    }

    pub fn join(&self, conn: u64, tx: Tx, code: &str, name: &str) -> Option<String> {
        let code = code.trim().to_uppercase();
        let mut rooms = self.inner.lock().unwrap();
        let Some(room) = rooms.get_mut(&code) else {
            error(&tx, format!("room {code} does not exist"));
            return None;
        };
        if room.game.is_some() || room.members.len() >= room.capacity {
            error(&tx, format!("room {code} is full"));
            return None;
        }
        let seat = room.members.len() + 1;
        room.members.push(Member { conn, name: clean_name(name, seat), tx, connected: true });
        if room.members.len() == room.capacity {
            room.game = Some(Game::new(Player::for_count(room.capacity)));
            room.broadcast_state(&code, Vec::new());
        } else {
            room.broadcast_lobby(&code);
        }
        Some(code)
    }

    /// Run a game action for `conn`, checking it is that player's turn.
    pub fn act(&self, code: &str, conn: u64, tx: &Tx, action: impl FnOnce(&mut Game) -> Result<Vec<Event>, ludo_core::Error>) {
        let mut rooms = self.inner.lock().unwrap();
        let Some(room) = rooms.get_mut(code) else { return };
        let (Some(seat), Some(game)) = (room.seat_of(conn), room.game.as_mut()) else {
            return error(tx, "the game has not started");
        };
        if game.turn != seat {
            return error(tx, "it is not your turn");
        }
        match action(game) {
            Ok(mut events) => {
                events.extend(room.autoplay());
                room.broadcast_state(code, events);
            }
            Err(e) => error(tx, e.to_string()),
        }
    }

    pub fn disconnect(&self, code: &str, conn: u64) {
        let mut rooms = self.inner.lock().unwrap();
        let Some(room) = rooms.get_mut(code) else { return };
        if room.game.is_none() {
            room.members.retain(|m| m.conn != conn);
            if room.members.is_empty() {
                rooms.remove(code);
            } else {
                room.broadcast_lobby(code);
            }
            return;
        }
        if let Some(seat) = room.seat_of(conn) {
            room.members[seat].connected = false;
        }
        if room.members.iter().all(|m| !m.connected) {
            rooms.remove(code);
        } else {
            let events = room.autoplay();
            room.broadcast_state(code, events);
        }
    }
}
