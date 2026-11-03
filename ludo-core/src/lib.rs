//! Ludo rules engine and shared network protocol.
//!
//! The engine is deterministic: dice values are passed in, so the server can
//! use real randomness while tests (and the same-screen client) stay simple.
//!
//! Token progress: `-1` = in the yard, `0..=50` = on the shared track relative
//! to the owner's start cell, `51..=55` = home column, `56` = home.

pub mod board;
pub mod protocol;

use serde::{Deserialize, Serialize};
use std::fmt;

pub const TOKENS: usize = 4;
pub const YARD: i8 = -1;
pub const LAST_TRACK: i8 = 50;
pub const HOME: i8 = 56;
pub const TRACK_LEN: usize = 52;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub enum Player {
    Red,
    Green,
    Yellow,
    Blue,
}

impl Player {
    /// Clockwise turn order.
    pub const ALL: [Player; 4] = [Player::Red, Player::Green, Player::Yellow, Player::Blue];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Player::Red => "Red",
            Player::Green => "Green",
            Player::Yellow => "Yellow",
            Player::Blue => "Blue",
        }
    }

    /// Index on the 52-cell track where this player's tokens enter.
    pub fn start_index(self) -> usize {
        self.index() * 13
    }

    /// Players for a game of `n` (2..=4), spread around the board.
    pub fn for_count(n: usize) -> Vec<Player> {
        match n {
            2 => vec![Player::Red, Player::Yellow],
            3 => vec![Player::Red, Player::Green, Player::Yellow],
            _ => Player::ALL.to_vec(),
        }
    }
}

/// Track cell (0..52) a token occupies, if it is on the shared track.
pub fn track_cell(player: Player, progress: i8) -> Option<usize> {
    (0..=LAST_TRACK)
        .contains(&progress)
        .then(|| (player.start_index() + progress as usize) % TRACK_LEN)
}

/// Start cells and the star cells 8 steps after them are safe from capture.
pub fn is_safe(cell: usize) -> bool {
    cell % 13 == 0 || cell % 13 == 8
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Phase {
    Roll,
    Move,
    Over,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Event {
    Rolled { seat: usize, value: u8 },
    NoMoves { seat: usize },
    TripleSix { seat: usize },
    Moved { seat: usize, token: usize, from: i8, to: i8 },
    /// `seat` is the victim.
    Captured { seat: usize, token: usize },
    Home { seat: usize, token: usize },
    ExtraTurn { seat: usize },
    Turn { seat: usize },
    Won { seat: usize },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    WrongPhase,
    BadDice,
    IllegalMove,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::WrongPhase => "not allowed right now",
            Error::BadDice => "dice value must be 1..=6",
            Error::IllegalMove => "that token cannot move",
        })
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Game {
    /// Participating players in turn order; a "seat" is an index into this.
    pub seats: Vec<Player>,
    /// Progress of each seat's four tokens.
    pub tokens: Vec<[i8; TOKENS]>,
    pub turn: usize,
    /// Rolled value waiting for a token to be chosen.
    pub dice: Option<u8>,
    /// Consecutive sixes rolled this turn.
    pub sixes: u8,
    pub winner: Option<usize>,
}

impl Game {
    pub fn new(seats: Vec<Player>) -> Self {
        assert!((2..=4).contains(&seats.len()), "Ludo needs 2 to 4 players");
        Game {
            tokens: vec![[YARD; TOKENS]; seats.len()],
            seats,
            turn: 0,
            dice: None,
            sixes: 0,
            winner: None,
        }
    }

    pub fn phase(&self) -> Phase {
        if self.winner.is_some() {
            Phase::Over
        } else if self.dice.is_some() {
            Phase::Move
        } else {
            Phase::Roll
        }
    }

    pub fn current(&self) -> Player {
        self.seats[self.turn]
    }

    fn destination(&self, seat: usize, token: usize, dice: u8) -> Option<i8> {
        match self.tokens[seat][token] {
            YARD => (dice == 6).then_some(0),
            HOME => None,
            p => Some(p + dice as i8).filter(|&t| t <= HOME),
        }
    }

    /// Tokens of the current player that can use the rolled value.
    pub fn movable(&self) -> Vec<usize> {
        match self.dice {
            Some(d) if self.winner.is_none() => (0..TOKENS)
                .filter(|&t| self.destination(self.turn, t, d).is_some())
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn roll(&mut self, value: u8) -> Result<Vec<Event>, Error> {
        if self.phase() != Phase::Roll {
            return Err(Error::WrongPhase);
        }
        if !(1..=6).contains(&value) {
            return Err(Error::BadDice);
        }
        let seat = self.turn;
        let mut events = vec![Event::Rolled { seat, value }];
        self.sixes = if value == 6 { self.sixes + 1 } else { 0 };
        if self.sixes == 3 {
            events.push(Event::TripleSix { seat });
            self.end_turn(&mut events);
            return Ok(events);
        }
        self.dice = Some(value);
        if self.movable().is_empty() {
            events.push(Event::NoMoves { seat });
            self.end_turn(&mut events);
        }
        Ok(events)
    }

    pub fn play(&mut self, token: usize) -> Result<Vec<Event>, Error> {
        let dice = match (self.phase(), self.dice) {
            (Phase::Move, Some(d)) => d,
            _ => return Err(Error::WrongPhase),
        };
        let seat = self.turn;
        let player = self.seats[seat];
        let to = (token < TOKENS)
            .then(|| self.destination(seat, token, dice))
            .flatten()
            .ok_or(Error::IllegalMove)?;
        let from = self.tokens[seat][token];
        self.tokens[seat][token] = to;
        let mut events = vec![Event::Moved { seat, token, from, to }];

        // A six, a capture or reaching home earns another roll.
        let mut bonus = dice == 6;

        if let Some(cell) = track_cell(player, to)
            && !is_safe(cell)
        {
            for other in (0..self.seats.len()).filter(|&s| s != seat) {
                for t in 0..TOKENS {
                    if track_cell(self.seats[other], self.tokens[other][t]) == Some(cell) {
                        self.tokens[other][t] = YARD;
                        events.push(Event::Captured { seat: other, token: t });
                        bonus = true;
                    }
                }
            }
        }

        self.dice = None;
        if to == HOME {
            events.push(Event::Home { seat, token });
            bonus = true;
            if self.tokens[seat].iter().all(|&p| p == HOME) {
                self.winner = Some(seat);
                events.push(Event::Won { seat });
                return Ok(events);
            }
        }
        if bonus {
            events.push(Event::ExtraTurn { seat });
        } else {
            self.end_turn(&mut events);
        }
        Ok(events)
    }

    fn end_turn(&mut self, events: &mut Vec<Event>) {
        self.sixes = 0;
        self.dice = None;
        self.turn = (self.turn + 1) % self.seats.len();
        events.push(Event::Turn { seat: self.turn });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> Game {
        Game::new(Player::for_count(4))
    }

    #[test]
    fn needs_six_to_leave_yard() {
        let mut g = game();
        let ev = g.roll(3).unwrap();
        assert!(ev.contains(&Event::NoMoves { seat: 0 }));
        assert_eq!(g.turn, 1);
        g.roll(6).unwrap();
        assert_eq!(g.movable(), vec![0, 1, 2, 3]);
        g.play(2).unwrap();
        assert_eq!(g.tokens[1][2], 0);
        assert_eq!(g.turn, 1, "a six grants another roll");
    }

    #[test]
    fn triple_six_forfeits_turn() {
        let mut g = game();
        g.roll(6).unwrap();
        g.play(0).unwrap();
        g.roll(6).unwrap();
        g.play(0).unwrap();
        let ev = g.roll(6).unwrap();
        assert!(ev.contains(&Event::TripleSix { seat: 0 }));
        assert_eq!(g.turn, 1);
        assert_eq!(g.phase(), Phase::Roll);
    }

    #[test]
    fn capture_sends_token_to_yard_and_grants_bonus() {
        let mut g = game();
        g.tokens[0][0] = 4; // red on cell 4
        // Green progress p sits on cell (13 + p) % 52; want cell 5 => p = 44.
        g.tokens[1][0] = 44;
        g.roll(1).unwrap();
        let ev = g.play(0).unwrap();
        assert!(ev.contains(&Event::Captured { seat: 1, token: 0 }));
        assert_eq!(g.tokens[1][0], YARD);
        assert_eq!(g.turn, 0, "capture grants another roll");
    }

    #[test]
    fn safe_cells_cannot_be_captured() {
        let mut g = game();
        g.tokens[0][0] = 7; // red on cell 7; cell 8 is a star
        // Yellow progress p sits on cell (26 + p) % 52; want cell 8 => p = 34.
        g.tokens[2][0] = 34;
        g.roll(1).unwrap();
        let ev = g.play(0).unwrap();
        assert!(!ev.iter().any(|e| matches!(e, Event::Captured { .. })));
        assert_eq!(g.tokens[2][0], 34);
        assert_eq!(g.turn, 1);
    }

    #[test]
    fn exact_roll_needed_to_finish_and_last_home_wins() {
        let mut g = Game::new(Player::for_count(2));
        g.tokens[0] = [HOME, HOME, HOME, 53];
        g.roll(4).unwrap();
        assert_eq!(g.phase(), Phase::Roll, "overshoot: no moves, turn passes");
        assert_eq!(g.turn, 1);
        g.roll(1).unwrap(); // seat 1 has nothing out
        assert_eq!(g.turn, 0);
        g.roll(3).unwrap();
        let ev = g.play(3).unwrap();
        assert!(ev.contains(&Event::Won { seat: 0 }));
        assert_eq!(g.phase(), Phase::Over);
        assert_eq!(g.roll(2), Err(Error::WrongPhase));
    }

    #[test]
    fn rejects_illegal_actions() {
        let mut g = game();
        assert_eq!(g.play(0), Err(Error::WrongPhase));
        assert_eq!(g.roll(7), Err(Error::BadDice));
        g.roll(6).unwrap();
        assert_eq!(g.roll(2), Err(Error::WrongPhase));
        assert_eq!(g.play(9), Err(Error::IllegalMove));
    }
}
