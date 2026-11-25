//! Client-side session: which screen we are on, the player's profile, the room
//! browser and the game being shown.
//!
//! In same-screen play the `Game` is driven locally with the shared rules
//! engine. In a room the server owns the game and we only mirror its
//! snapshots and send requests.

use crate::{
    net::{self, NetEvent},
    store::{self, Profile, Settings},
};
use bevy::prelude::Resource;
use ludo_core::{
    Event, Game, Phase, Player,
    protocol::{ClientMsg, LobbyPlayer, RoomInfo, ServerMsg},
};

pub const ROOMS_PER_PAGE: usize = 3;
pub const NAME_MAX: usize = 16;
pub const ROOM_NAME_MAX: usize = 24;
/// Why we are waiting for the server to give us our seat back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Resume {
    /// The connection dropped while we were in a room.
    Reconnecting,
    /// The page was loaded again while we were in a room.
    Rejoining,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Profile,
    Rooms,
    Create,
    Room,
    Game,
}

/// Everything the user can ask for from the UI.
#[derive(Clone, Copy, Debug)]
pub enum Action {
    SaveProfile,
    OpenProfile,
    /// Pick a colour (in the profile, or in the waiting room).
    PickColor(Player),
    OpenCreate,
    SetPlayers(u8),
    CreateRoom,
    /// Join the room shown on this row of the current page.
    JoinRow(usize),
    PageNext,
    PagePrev,
    /// Host: start the game.
    Start,
    /// Start a same-screen game with this many players.
    Local(u8),
    Roll,
    ToggleEffects,
    ToggleReducedMotion,
    /// Leave the room or game and go back to the room list.
    Leave,
    Retry,
    Back,
}

/// The waiting room the player is in.
pub struct RoomView {
    pub name: String,
    pub host: usize,
    pub capacity: usize,
}

#[derive(Resource)]
pub struct Session {
    pub screen: Screen,
    pub profile: Profile,
    pub has_profile: bool,
    pub settings: Settings,
    /// Text being edited on the profile / create screens (kept in sync by `ui`).
    pub name_draft: String,
    pub room_draft: String,
    pub color_draft: Player,
    pub create_players: u8,

    pub connected: bool,
    /// The connection has failed and is being retried.
    pub lost: bool,
    /// Set until the server says whether we still have our seat.
    pub resume: Option<Resume>,
    /// We asked to leave and the server has not confirmed it yet.
    leaving: bool,
    /// The room remembered in local storage.
    saved_room: Option<u32>,
    pub rooms: Vec<RoomInfo>,
    pub page: usize,

    /// The game is run by the server (as opposed to same-screen play).
    pub in_room: bool,
    pub room: Option<RoomView>,
    pub game: Option<Game>,
    pub players: Vec<LobbyPlayer>,
    pub you: Option<usize>,
    pub last_roll: Option<u8>,
    pub last_event: Option<String>,
    /// Game events not yet handed to the token animations.
    pub pending: Vec<Event>,
    /// Tokens are still moving; the player cannot act until they land.
    pub animating: bool,
    /// The die is still tumbling; the player cannot act until it settles.
    pub dice_busy: bool,
    /// Our roll is on its way to the server.
    pub rolling: bool,
    /// How many rolls have been seen, and by which seat the last one was.
    pub roll_count: u32,
    pub last_roller: Option<usize>,
    pub error: Option<String>,
    /// Seconds before the server plays the current turn, and a counter that
    /// changes whenever the server sends a fresh value.
    pub turn_secs: Option<f32>,
    pub turn_rev: u32,
    /// Bumped when a new game starts (token entities are rebuilt).
    pub generation: u32,
    /// Bumped whenever the game state changes (tokens are re-targeted).
    pub rev: u32,
}

fn trimmed(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect()
}

impl Session {
    pub fn new() -> Self {
        let stored = store::load();
        let has_profile = stored.is_some();
        let profile = stored.unwrap_or_else(store::random_profile);
        let saved_room = store::load_room().filter(|_| has_profile);
        Session {
            screen: if has_profile {
                Screen::Rooms
            } else {
                Screen::Profile
            },
            name_draft: profile.name.clone(),
            color_draft: profile.color,
            profile,
            has_profile,
            settings: store::load_settings(),
            room_draft: String::new(),
            create_players: 4,
            connected: false,
            lost: false,
            resume: saved_room.map(|_| Resume::Rejoining),
            leaving: false,
            saved_room,
            rooms: Vec::new(),
            page: 0,
            in_room: false,
            room: None,
            game: None,
            players: Vec::new(),
            you: None,
            last_roll: None,
            last_event: None,
            pending: Vec::new(),
            animating: false,
            dice_busy: false,
            rolling: false,
            roll_count: 0,
            last_roller: None,
            error: None,
            turn_secs: None,
            turn_rev: 0,
            generation: 0,
            rev: 0,
        }
    }

    // ---- queries -----------------------------------------------------------

    /// Can the local user act on the current game state?
    pub fn can_act(&self) -> bool {
        self.game.as_ref().is_some_and(|g| {
            g.phase() != Phase::Over
                && !self.animating
                && !self.dice_busy
                && !self.rolling
                && (!self.in_room || (self.resume.is_none() && self.you == Some(g.turn)))
        })
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

    pub fn name(&self, seat: usize) -> &str {
        self.players.get(seat).map_or("?", |p| p.name.as_str())
    }

    pub fn is_host(&self) -> bool {
        self.room.as_ref().is_some_and(|r| Some(r.host) == self.you)
    }

    pub fn page_count(&self) -> usize {
        self.rooms.len().div_ceil(ROOMS_PER_PAGE).max(1)
    }

    /// The room on this row of the current page.
    pub fn row(&self, row: usize) -> Option<&RoomInfo> {
        self.rooms.get(self.page * ROOMS_PER_PAGE + row)
    }

    pub fn status(&self) -> String {
        let Some(g) = &self.game else {
            return String::new();
        };
        let mine = !self.in_room || self.you == Some(g.turn);
        let who = self.name(g.turn);
        match (g.phase(), mine, self.in_room) {
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
            Action::SaveProfile => self.save_profile(),
            Action::OpenProfile => {
                self.name_draft = self.profile.name.clone();
                self.color_draft = self.profile.color;
                self.error = None;
                self.screen = Screen::Profile;
            }
            Action::PickColor(color) => {
                if self.screen == Screen::Room {
                    net::send(&ClientMsg::SetColor { color });
                } else {
                    self.color_draft = color;
                }
            }
            Action::OpenCreate => {
                self.room_draft = format!("{}'s room", self.profile.name);
                self.error = None;
                self.screen = Screen::Create;
            }
            Action::SetPlayers(n) => self.create_players = n.clamp(2, 4),
            Action::CreateRoom => {
                if self.connected {
                    self.leaving = false;
                    let name = trimmed(&self.room_draft, ROOM_NAME_MAX);
                    net::send(&ClientMsg::CreateRoom {
                        name,
                        players: self.create_players,
                    });
                } else {
                    self.error = Some("Not connected to the server".into());
                }
            }
            Action::JoinRow(row) => {
                if let Some(id) = self.row(row).map(|r| r.id) {
                    self.leaving = false;
                    net::send(&ClientMsg::JoinRoom { id });
                }
            }
            Action::PageNext => self.page = (self.page + 1).min(self.page_count() - 1),
            Action::PagePrev => self.page = self.page.saturating_sub(1),
            Action::Start => net::send(&ClientMsg::StartGame),
            Action::Local(n) => {
                self.clear_game();
                self.resume = None;
                let seats = Player::for_count(n as usize);
                self.players = seats
                    .iter()
                    .map(|&p| LobbyPlayer {
                        name: p.name().into(),
                        player: p,
                        connected: true,
                        idle: false,
                        bot: false,
                    })
                    .collect();
                self.start_game(Game::new(seats));
            }
            Action::Roll => self.roll(),
            Action::ToggleEffects => {
                self.settings.effects = !self.settings.effects;
                store::save_settings(&self.settings);
            }
            Action::ToggleReducedMotion => {
                self.settings.reduced_motion = !self.settings.reduced_motion;
                store::save_settings(&self.settings);
            }
            Action::Leave => {
                if self.in_room {
                    // Repeated after a reconnect until the server confirms.
                    self.leaving = true;
                    net::send(&ClientMsg::LeaveRoom);
                }
                self.resume = None;
                self.remember(None);
                self.clear_game();
                self.error = None;
                self.screen = Screen::Rooms;
            }
            Action::Retry => self.connect(),
            Action::Back => {
                self.error = None;
                self.screen = Screen::Rooms;
            }
        }
    }

    fn save_profile(&mut self) {
        let name = trimmed(&self.name_draft, NAME_MAX);
        self.profile.name = if name.is_empty() {
            store::random_profile().name
        } else {
            name
        };
        self.profile.color = self.color_draft;
        store::save(&self.profile);
        self.has_profile = true;
        self.error = None;
        if self.connected {
            net::send(&self.hello());
        } else {
            self.connect();
        }
        self.screen = Screen::Rooms;
    }

    fn hello(&self) -> ClientMsg {
        ClientMsg::Hello {
            client_id: self.profile.client_id.clone(),
            name: self.profile.name.clone(),
            color: self.profile.color,
            heartbeat: true,
        }
    }

    /// Connect to the server, or retry now.
    pub fn connect(&mut self) {
        net::start();
    }

    fn remember(&mut self, room: Option<u32>) {
        if self.saved_room != room {
            self.saved_room = room;
            store::save_room(room);
        }
    }

    fn clear_game(&mut self) {
        self.in_room = false;
        self.room = None;
        self.game = None;
        self.players.clear();
        self.you = None;
        self.last_roll = None;
        self.last_event = None;
        self.pending.clear();
        self.animating = false;
        self.dice_busy = false;
        self.rolling = false;
        self.last_roller = None;
        self.turn_secs = None;
        self.generation += 1;
        self.rev += 1;
    }

    fn start_game(&mut self, game: Game) {
        self.game = Some(game);
        self.screen = Screen::Game;
        self.pending.clear();
        self.last_event = None;
        self.last_roll = None;
        self.last_roller = None;
        self.rolling = false;
        self.generation += 1;
        self.rev += 1;
    }

    pub fn roll(&mut self) {
        if !self.can_roll() {
            return;
        }
        if self.in_room {
            self.rolling = true;
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
        if self.in_room {
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
            NetEvent::Open => {
                self.connected = true;
                self.lost = false;
                self.error = None;
                net::send(&self.hello());
                if self.leaving {
                    net::send(&ClientMsg::LeaveRoom);
                }
            }
            NetEvent::Closed => self.disconnected(),
            NetEvent::Msg(msg) => self.handle_msg(msg),
        }
    }

    /// Keep what is on screen and wait to be given the seat back.
    fn disconnected(&mut self) {
        self.connected = false;
        self.lost = true;
        self.rooms.clear();
        self.rolling = false;
        self.turn_secs = None;
        self.turn_rev += 1;
        if !self.in_room {
            return;
        }
        if self.game.as_ref().is_some_and(|g| g.phase() == Phase::Over) {
            // Nothing to return to; leave the result on screen.
            self.in_room = false;
        } else {
            self.resume = Some(Resume::Reconnecting);
        }
    }

    fn handle_msg(&mut self, msg: ServerMsg) {
        match msg {
            ServerMsg::Rooms { rooms } => {
                self.leaving = false;
                // The room list instead of a snapshot: our seat is gone.
                if self.resume.take().is_some() {
                    if self.in_room {
                        self.clear_game();
                        self.screen = Screen::Rooms;
                    }
                    self.remember(None);
                    self.error = Some("Your game is no longer available".into());
                }
                self.rooms = rooms;
                self.page = self.page.min(self.page_count() - 1);
            }
            // Sent before the server saw us leave.
            ServerMsg::Room { .. } | ServerMsg::State { .. } if self.leaving => {}
            ServerMsg::Room {
                id,
                name,
                you,
                host,
                capacity,
                players,
            } => {
                self.resume = None;
                self.remember(Some(id));
                self.in_room = true;
                self.room = Some(RoomView {
                    name,
                    host,
                    capacity,
                });
                self.you = Some(you);
                self.players = players;
                self.error = None;
                self.screen = Screen::Room;
            }
            ServerMsg::State {
                id,
                you,
                players,
                game,
                events,
                turn_ms,
            } => {
                self.resume = None;
                self.remember((game.phase() != Phase::Over).then_some(id));
                self.in_room = true;
                self.you = Some(you);
                self.players = players;
                self.error = None;
                self.turn_secs = turn_ms.map(|ms| ms as f32 / 1000.0);
                self.turn_rev += 1;
                if self.screen != Screen::Game {
                    self.start_game(game);
                } else {
                    self.game = Some(game);
                }
                self.apply_events(events);
            }
            // Answered as it arrives, in `net`.
            ServerMsg::Ping => {}
            ServerMsg::Removed { reason } => {
                self.resume = None;
                self.leaving = false;
                self.remember(None);
                self.clear_game();
                self.screen = Screen::Rooms;
                self.error = Some(reason);
            }
            ServerMsg::Error { message } => {
                self.rolling = false;
                self.error = Some(message);
            }
        }
    }

    fn apply_events(&mut self, events: Vec<Event>) {
        self.pending.extend(events.iter().cloned());
        for e in &events {
            if let Event::Rolled { seat, value } = e {
                self.last_roll = Some(*value);
                self.last_roller = Some(*seat);
                self.roll_count += 1;
                self.rolling = false;
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
            Event::Left { seat } => format!("{} left the game", self.name(seat)),
            Event::Won { seat } => format!("{} wins!", self.name(seat)),
            Event::Moved { .. } | Event::Turn { .. } => return None,
        })
    }
}

pub fn random_die() -> u8 {
    (js_sys::Math::random() * 6.0) as u8 % 6 + 1
}
