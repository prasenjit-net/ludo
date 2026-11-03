//! Messages exchanged between the client and the server (JSON over WebSocket).

use crate::{Event, Game, Player};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Introduce this client. Sent on every (re)connect and when the profile changes.
    /// `client_id` is stable across reconnects and lets the server restore a seat.
    Hello {
        client_id: String,
        name: String,
        color: Player,
    },
    /// Open a room for `players` (2..=4) and take the first seat as host.
    CreateRoom {
        name: String,
        players: u8,
    },
    JoinRoom {
        id: u32,
    },
    /// Switch colour while waiting in a room (ignored if taken).
    SetColor {
        color: Player,
    },
    /// Host only: start now; seats nobody has taken are filled with AI players.
    StartGame,
    LeaveRoom,
    Roll,
    Play {
        token: usize,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LobbyPlayer {
    pub name: String,
    pub player: Player,
    pub connected: bool,
    /// Played by the server, not by a person.
    #[serde(default)]
    pub bot: bool,
}

/// One row of the room browser.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoomInfo {
    pub id: u32,
    pub name: String,
    pub host: String,
    pub capacity: usize,
    pub joined: usize,
    /// A game is in progress; the room cannot be joined.
    pub playing: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Open rooms, pushed to everyone who is not in a room.
    Rooms {
        rooms: Vec<RoomInfo>,
    },
    /// The waiting room you are in. `you` and `host` are indices into `players`.
    Room {
        id: u32,
        name: String,
        you: usize,
        host: usize,
        capacity: usize,
        players: Vec<LobbyPlayer>,
    },
    /// Authoritative game snapshot plus what just happened. `turn_ms` is the
    /// time left before the server plays the current turn automatically.
    State {
        id: u32,
        you: usize,
        players: Vec<LobbyPlayer>,
        game: Game,
        events: Vec<Event>,
        turn_ms: Option<u32>,
    },
    /// You were removed from your room (e.g. for being inactive).
    Removed {
        reason: String,
    },
    Error {
        message: String,
    },
}
