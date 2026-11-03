//! Client-side session: which screen we are on and the game being shown.
//!
//! In same-screen play the `Game` is driven locally with the shared rules
//! engine. In online play the server owns the game and we only mirror its
//! snapshots and send requests.

use crate::{bridge::{self, Cmd}, net::{self, NetEvent}};
use bevy::prelude::Resource;
use ludo_core::{
    Event, Game, Phase, Player,
    protocol::{ClientMsg, LobbyPlayer, ServerMsg},
};
use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Screen {
    Menu,
    Lobby,
    Game,
}

#[derive(Resource)]
pub struct Session {
    pub screen: Screen,
    online: bool,
    pub game: Option<Game>,
    players: Vec<LobbyPlayer>,
    capacity: usize,
    you: Option<usize>,
    room: Option<String>,
    log: Vec<String>,
    last_roll: Option<(usize, u8)>,
    error: Option<String>,
    /// Bumped when a new game starts (token entities are rebuilt).
    pub generation: u32,
    /// Bumped whenever the game state changes (tokens are re-targeted).
    pub rev: u32,
    ui_dirty: bool,
}

#[derive(Serialize)]
struct UiPlayer<'a> {
    name: &'a str,
    color: &'static str,
    connected: bool,
}

#[derive(Serialize)]
struct UiState<'a> {
    screen: Screen,
    online: bool,
    room: &'a Option<String>,
    capacity: usize,
    players: Vec<UiPlayer<'a>>,
    you: Option<usize>,
    turn: Option<usize>,
    dice: Option<u8>,
    can_roll: bool,
    winner: Option<usize>,
    status: String,
    log: &'a [String],
    error: &'a Option<String>,
}

impl Session {
    pub fn new() -> Self {
        Session {
            screen: Screen::Menu,
            online: false,
            game: None,
            players: Vec::new(),
            capacity: 0,
            you: None,
            room: None,
            log: Vec::new(),
            last_roll: None,
            error: None,
            generation: 0,
            rev: 0,
            ui_dirty: true,
        }
    }

    /// Can the local user act on the current game state?
    pub fn can_act(&self) -> bool {
        match &self.game {
            Some(g) => g.phase() != Phase::Over && (!self.online || self.you == Some(g.turn)),
            None => false,
        }
    }

    pub fn can_roll(&self) -> bool {
        self.can_act() && self.game.as_ref().is_some_and(|g| g.phase() == Phase::Roll)
    }

    /// Tokens the local user may click right now: `(seat, token)`.
    pub fn clickable(&self) -> Option<(usize, Vec<usize>)> {
        let g = self.game.as_ref()?;
        (self.can_act() && g.phase() == Phase::Move).then(|| (g.turn, g.movable()))
    }

    fn reset(&mut self) {
        net::close();
        *self = Session { generation: self.generation + 1, rev: self.rev + 1, ..Session::new() };
    }

    fn start_game(&mut self, game: Game) {
        self.game = Some(game);
        self.screen = Screen::Game;
        self.log.clear();
        self.last_roll = None;
        self.generation += 1;
        self.rev += 1;
        self.ui_dirty = true;
    }

    // ---- commands from the HTML layer -------------------------------------

    pub fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Local { players } => {
                self.reset();
                let n = players.clamp(2, 4) as usize;
                let seats = Player::for_count(n);
                self.players = seats
                    .iter()
                    .map(|&p| LobbyPlayer { name: p.name().into(), player: p, connected: true })
                    .collect();
                self.start_game(Game::new(seats));
            }
            Cmd::Create { name, players } => self.go_online(ClientMsg::Create { name, players }),
            Cmd::Join { name, code } => self.go_online(ClientMsg::Join { name, code }),
            Cmd::Roll => self.roll(),
            Cmd::Menu => self.reset(),
        }
    }

    fn go_online(&mut self, first: ClientMsg) {
        self.reset();
        match net::connect(&first) {
            Ok(()) => self.online = true,
            Err(e) => self.error = Some(e),
        }
    }

    // ---- player actions ----------------------------------------------------

    pub fn roll(&mut self) {
        if !self.can_roll() {
            return;
        }
        if self.online {
            net::send(&ClientMsg::Roll);
        } else if let Some(game) = &mut self.game
            && let Ok(events) = game.roll(bridge::random_die())
        {
            self.apply(events);
        }
    }

    pub fn play(&mut self, token: usize) {
        if !self.can_act() {
            return;
        }
        if self.online {
            net::send(&ClientMsg::Play { token });
        } else if let Some(game) = &mut self.game
            && let Ok(events) = game.play(token)
        {
            self.apply(events);
        }
    }

    // ---- server messages ---------------------------------------------------

    pub fn handle_net(&mut self, ev: NetEvent) {
        self.ui_dirty = true;
        match ev {
            NetEvent::Msg(ServerMsg::Lobby { code, you, capacity, players }) => {
                self.screen = Screen::Lobby;
                self.room = Some(code);
                self.you = Some(you);
                self.capacity = capacity;
                self.players = players;
                self.error = None;
            }
            NetEvent::Msg(ServerMsg::State { code, you, players, game, events }) => {
                self.room = Some(code);
                self.you = Some(you);
                self.players = players;
                self.error = None;
                if self.screen != Screen::Game {
                    self.start_game(game);
                } else {
                    self.game = Some(game);
                }
                self.apply(events);
            }
            NetEvent::Msg(ServerMsg::Error { message }) => {
                self.error = Some(message);
                if self.room.is_none() {
                    net::close();
                    self.online = false;
                }
            }
            NetEvent::Closed => {
                if self.screen != Screen::Menu || self.online {
                    let msg = "Disconnected from the server".to_string();
                    self.reset();
                    self.error = Some(msg);
                }
            }
        }
    }

    fn apply(&mut self, events: Vec<Event>) {
        for e in &events {
            if let Event::Rolled { seat, value } = e {
                self.last_roll = Some((*seat, *value));
            }
            if let Some(line) = self.describe(e) {
                self.log.push(line);
            }
        }
        let excess = self.log.len().saturating_sub(40);
        self.log.drain(..excess);
        self.rev += 1;
        self.ui_dirty = true;
    }

    fn name(&self, seat: usize) -> String {
        self.players.get(seat).map_or_else(|| "?".into(), |p| p.name.clone())
    }

    fn describe(&self, e: &Event) -> Option<String> {
        Some(match *e {
            Event::Rolled { seat, value } => format!("{} rolled {value}", self.name(seat)),
            Event::NoMoves { .. } => "No move possible".into(),
            Event::TripleSix { .. } => "Three sixes in a row: turn lost".into(),
            Event::Captured { seat, .. } => format!("{}'s token was captured!", self.name(seat)),
            Event::Home { seat, .. } => format!("{} got a token home", self.name(seat)),
            Event::ExtraTurn { seat } => format!("{} rolls again", self.name(seat)),
            Event::Won { seat } => format!("{} wins!", self.name(seat)),
            Event::Moved { .. } | Event::Turn { .. } => return None,
        })
    }

    // ---- UI state ----------------------------------------------------------

    fn status(&self) -> String {
        let Some(g) = &self.game else { return String::new() };
        let who = self.name(g.turn);
        let mine = !self.online || self.you == Some(g.turn);
        match (g.phase(), mine) {
            (Phase::Over, _) => format!("{} wins!", self.name(g.winner.unwrap_or(0))),
            (Phase::Roll, true) if self.online => "Your turn: roll the dice".into(),
            (Phase::Move, true) if self.online => "Your turn: pick a token".into(),
            (Phase::Roll, true) => format!("{who}: roll the dice"),
            (Phase::Move, true) => format!("{who}: pick a token to move"),
            (_, false) => format!("Waiting for {who}…"),
        }
    }

    pub fn push_ui_if_dirty(&mut self) {
        if !std::mem::take(&mut self.ui_dirty) {
            return;
        }
        let dice = self
            .game
            .as_ref()
            .and_then(|g| g.dice)
            .or(self.last_roll.map(|(_, v)| v));
        let state = UiState {
            screen: self.screen,
            online: self.online,
            room: &self.room,
            capacity: self.capacity,
            players: self
                .players
                .iter()
                .map(|p| UiPlayer { name: &p.name, color: p.player.name(), connected: p.connected })
                .collect(),
            you: self.you,
            turn: self.game.as_ref().map(|g| g.turn),
            dice,
            can_roll: self.can_roll(),
            winner: self.game.as_ref().and_then(|g| g.winner),
            status: self.status(),
            log: &self.log,
            error: &self.error,
        };
        if let Ok(json) = serde_json::to_string(&state) {
            bridge::render(&json);
        }
    }
}
