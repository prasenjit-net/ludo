//! Messages exchanged between the client and the server (JSON over WebSocket).

use crate::{Event, Game, Player};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Open a new room for `players` (2..=4) and take the first seat.
    Create { name: String, players: u8 },
    Join { code: String, name: String },
    Roll,
    Play { token: usize },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LobbyPlayer {
    pub name: String,
    pub player: Player,
    pub connected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Waiting for more players. `you` is this client's seat.
    Lobby {
        code: String,
        you: usize,
        capacity: usize,
        players: Vec<LobbyPlayer>,
    },
    /// Authoritative game snapshot plus what just happened.
    State {
        code: String,
        you: usize,
        players: Vec<LobbyPlayer>,
        game: Game,
        events: Vec<Event>,
    },
    Error { message: String },
}
