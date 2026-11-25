//! Messages exchanged between the client and the server (JSON over WebSocket).

use crate::{Event, Game, Player};
use serde::{Deserialize, Serialize};

/// Raise this when a change to these messages would confuse an older client:
/// the client reloads itself when the server's number differs from its own.
/// `Welcome` itself has to stay readable by older clients.
pub const PROTOCOL: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Introduce this client. Sent on every (re)connect and when the profile changes.
    /// `client_id` is stable across reconnects and lets the server restore a seat.
    Hello {
        client_id: String,
        name: String,
        color: Player,
        /// The client answers `Ping`, so the server may drop it when it goes
        /// quiet. Clients from before heartbeats leave this out.
        #[serde(default)]
        heartbeat: bool,
        /// The room this client believes it is in, to be told what became of it.
        #[serde(default)]
        room: Option<u32>,
    },
    /// Answer to `ServerMsg::Ping`.
    Pong,
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
    /// `rev` is that of the `State` the player was looking at; the server
    /// rejects the request if the game has moved on.
    Roll {
        #[serde(default)]
        rev: Option<u64>,
    },
    Play {
        token: usize,
        #[serde(default)]
        rev: Option<u64>,
    },
}

/// What became of the room named in `Hello`, or the seat found for this client.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Resumed {
    /// Back in a game; its `State` follows.
    Game,
    /// Back in a waiting room; its `Room` follows.
    Lobby,
    /// The game ended while the client was away.
    Finished { winner: String },
    /// Nobody came back in time.
    Expired,
    /// The player had left that game.
    Left,
    /// The server knows nothing of it, for instance after a restart.
    NotFound,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    AlreadyInRoom,
    BadPlayerCount,
    RoomGone,
    GameStarted,
    RoomFull,
    NotHost,
    NotStarted,
    NotYourTurn,
    /// The rules do not allow that move.
    NotAllowed,
    /// The game moved on before the request arrived.
    Stale,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LobbyPlayer {
    pub name: String,
    pub player: Player,
    pub connected: bool,
    /// Connected but not playing; a bot takes their turns until they act.
    #[serde(default)]
    pub idle: bool,
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
    /// The answer to the first `Hello` of a connection.
    Welcome {
        protocol: u32,
        resume: Option<Resumed>,
    },
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
        /// Counts the steps of the game; see `ClientMsg::Roll`.
        rev: u64,
    },
    /// Sent every few seconds to check that the connection is alive.
    Ping,
    /// You were removed from your room (e.g. it was opened in another tab).
    Removed {
        reason: String,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
    /// The server is shutting down; games in progress will be lost.
    Restarting,
}
