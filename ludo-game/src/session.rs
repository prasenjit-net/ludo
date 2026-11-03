//! Client-side session: which screen we are on and the game being shown.
//!
//! In same-screen play the `Game` is driven locally with the shared rules
//! engine. In online play the server owns the game and we only mirror its
//! snapshots and send requests.

use crate::net::{self, NetEvent};
use bevy::prelude::Resource;
use ludo_core::{
    Event, Game, Phase, Player,
    protocol::{ClientMsg, LobbyPlayer, ServerMsg},
};

pub const CODE_LEN: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Menu,
    Join,
    Lobby,
    Game,
}

/// Everything the user can ask for from the UI.
#[derive(Clone, Copy, Debug)]
pub enum Action {
    /// Start a same-screen game with this many players.
    Local(u8),
    /// Open an online room for this many players.
    Create(u8),
    OpenJoin,
    Digit(u8),
    Backspace,
    Join,
    Roll,
    Menu,
}

#[derive(Resource)]
pub struct Session {
    pub screen: Screen,
    pub online: bool,
    pub game: Option<Game>,
    pub players: Vec<LobbyPlayer>,
    pub capacity: usize,
    pub you: Option<usize>,
    pub room: Option<String>,
    pub join_code: String,
    pub last_roll: Option<u8>,
    pub last_event: Option<String>,
    pub error: Option<String>,
    /// Bumped when a new game starts (token entities are rebuilt).
    pub generation: u32,
    /// Bumped whenever the game state changes (tokens are re-targeted).
    pub rev: u32,
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
            join_code: String::new(),
            last_roll: None,
            last_event: None,
            error: None,
            generation: 0,
            rev: 0,
        }
    }

    /// Can the local user act on the current game state?
    pub fn can_act(&self) -> bool {
        self.game
            .as_ref()
            .is_some_and(|g| g.phase() != Phase::Over && (!self.online || self.you == Some(g.turn)))
    }

    pub fn can_roll(&self) -> bool {
        self.can_act() && self.game.as_ref().is_some_and(|g| g.phase() == Phase::Roll)
    }

    /// Tokens the local user may tap right now: `(seat, tokens)`.
    pub fn clickable(&self) -> Option<(usize, Vec<usize>)> {
        let g = self.game.as_ref()?;
        (self.can_act() && g.phase() == Phase::Move).then(|| (g.turn, g.movable()))
    }

    /// The die to show: the pending roll, else the last one rolled.
    pub fn die(&self) -> Option<u8> {
        self.game.as_ref().and_then(|g| g.dice).or(self.last_roll)
    }

    pub fn winner(&self) -> Option<usize> {
        self.game.as_ref().and_then(|g| g.winner)
    }

    pub fn name(&self, seat: usize) -> &'static str {
        self.players.get(seat).map_or("?", |p| p.player.name())
    }

    pub fn status(&self) -> String {
        let Some(g) = &self.game else {
            return String::new();
        };
        let mine = !self.online || self.you == Some(g.turn);
        let who = self.name(g.turn);
        match (g.phase(), mine, self.online) {
            (Phase::Over, ..) => format!("{} wins!", self.name(g.winner.unwrap_or(0))),
            (Phase::Roll, true, true) => "Your turn: roll the dice".into(),
            (Phase::Move, true, true) => "Your turn: tap a token".into(),
            (Phase::Roll, true, false) => format!("{who}: roll the dice"),
            (Phase::Move, true, false) => format!("{who}: tap a token"),
            (_, false, _) => format!("Waiting for {who}..."),
        }
    }

    // ---- actions -----------------------------------------------------------

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::Local(n) => {
                self.reset();
                let seats = Player::for_count(n as usize);
                self.players = seats
                    .iter()
                    .map(|&p| LobbyPlayer {
                        player: p,
                        connected: true,
                    })
                    .collect();
                self.start_game(Game::new(seats));
            }
            Action::Create(players) => self.go_online(ClientMsg::Create { players }),
            Action::OpenJoin => {
                self.reset();
                self.screen = Screen::Join;
            }
            Action::Digit(d) => {
                if self.join_code.len() < CODE_LEN {
                    self.join_code.push(char::from(b'0' + d));
                }
            }
            Action::Backspace => {
                self.join_code.pop();
            }
            Action::Join => {
                if self.join_code.len() == CODE_LEN {
                    let code = self.join_code.clone();
                    self.go_online(ClientMsg::Join { code });
                    self.screen = Screen::Join;
                }
            }
            Action::Roll => self.roll(),
            Action::Menu => self.reset(),
        }
    }

    fn reset(&mut self) {
        net::close();
        *self = Session {
            generation: self.generation + 1,
            rev: self.rev + 1,
            ..Session::new()
        };
    }

    fn go_online(&mut self, first: ClientMsg) {
        let code = std::mem::take(&mut self.join_code);
        self.reset();
        self.join_code = code;
        match net::connect(&first) {
            Ok(()) => self.online = true,
            Err(e) => self.error = Some(e),
        }
    }

    fn start_game(&mut self, game: Game) {
        self.game = Some(game);
        self.screen = Screen::Game;
        self.last_event = None;
        self.last_roll = None;
        self.generation += 1;
        self.rev += 1;
    }

    pub fn roll(&mut self) {
        if !self.can_roll() {
            return;
        }
        if self.online {
            net::send(&ClientMsg::Roll);
        } else if let Some(game) = &mut self.game
            && let Ok(events) = game.roll(random_die())
        {
            self.apply_events(events);
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
            self.apply_events(events);
        }
    }

    // ---- server messages ---------------------------------------------------

    pub fn handle_net(&mut self, ev: NetEvent) {
        match ev {
            NetEvent::Msg(ServerMsg::Lobby {
                code,
                you,
                capacity,
                players,
            }) => {
                self.screen = Screen::Lobby;
                self.room = Some(code);
                self.you = Some(you);
                self.capacity = capacity;
                self.players = players;
                self.error = None;
            }
            NetEvent::Msg(ServerMsg::State {
                code,
                you,
                players,
                game,
                events,
            }) => {
                self.room = Some(code);
                self.you = Some(you);
                self.players = players;
                self.error = None;
                if self.screen != Screen::Game {
                    self.start_game(game);
                } else {
                    self.game = Some(game);
                }
                self.apply_events(events);
            }
            NetEvent::Msg(ServerMsg::Error { message }) => {
                self.error = Some(message);
                if self.room.is_none() {
                    net::close();
                    self.online = false;
                }
            }
            NetEvent::Closed => {
                if self.online {
                    self.reset();
                    self.error = Some("Disconnected from the server".into());
                }
            }
        }
    }

    fn apply_events(&mut self, events: Vec<Event>) {
        for e in &events {
            if let Event::Rolled { value, .. } = e {
                self.last_roll = Some(*value);
            }
            if let Some(line) = self.describe(e) {
                self.last_event = Some(line);
            }
        }
        self.rev += 1;
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
}

fn random_die() -> u8 {
    (js_sys::Math::random() * 6.0) as u8 % 6 + 1
}
