//! Board geometry on a 15x15 grid. Cells are `(col, row)` with row 0 at the top.

use crate::{Event, HOME, LAST_TRACK, Player, YARD, track_cell};

pub const GRID: i32 = 15;

/// The 52 shared track cells, clockwise, starting at Red's start cell.
#[rustfmt::skip]
pub const TRACK: [(i32, i32); 52] = [
    (1, 6), (2, 6), (3, 6), (4, 6), (5, 6),
    (6, 5), (6, 4), (6, 3), (6, 2), (6, 1), (6, 0),
    (7, 0), (8, 0),
    (8, 1), (8, 2), (8, 3), (8, 4), (8, 5),
    (9, 6), (10, 6), (11, 6), (12, 6), (13, 6), (14, 6),
    (14, 7), (14, 8),
    (13, 8), (12, 8), (11, 8), (10, 8), (9, 8),
    (8, 9), (8, 10), (8, 11), (8, 12), (8, 13), (8, 14),
    (7, 14), (6, 14),
    (6, 13), (6, 12), (6, 11), (6, 10), (6, 9),
    (5, 8), (4, 8), (3, 8), (2, 8), (1, 8), (0, 8),
    (0, 7), (0, 6),
];

impl Player {
    /// Top-left grid cell of this player's 6x6 yard.
    pub fn yard_origin(self) -> (i32, i32) {
        match self {
            Player::Red => (0, 0),
            Player::Green => (9, 0),
            Player::Yellow => (9, 9),
            Player::Blue => (0, 9),
        }
    }

    /// The five coloured cells leading to the centre.
    pub fn home_column(self) -> [(i32, i32); 5] {
        match self {
            Player::Red => [(1, 7), (2, 7), (3, 7), (4, 7), (5, 7)],
            Player::Green => [(7, 1), (7, 2), (7, 3), (7, 4), (7, 5)],
            Player::Yellow => [(13, 7), (12, 7), (11, 7), (10, 7), (9, 7)],
            Player::Blue => [(7, 13), (7, 12), (7, 11), (7, 10), (7, 9)],
        }
    }

    /// Where a finished token rests, next to the centre.
    fn home_spot(self) -> (f32, f32) {
        match self {
            Player::Red => (6.45, 7.0),
            Player::Green => (7.0, 6.45),
            Player::Yellow => (7.55, 7.0),
            Player::Blue => (7.0, 7.55),
        }
    }

    fn yard_spot(self, token: usize) -> (f32, f32) {
        let (c, r) = self.yard_origin();
        let dx = if token.is_multiple_of(2) { -1.0 } else { 1.0 };
        let dy = if token < 2 { -1.0 } else { 1.0 };
        (c as f32 + 2.5 + dx, r as f32 + 2.5 + dy)
    }
}

/// Grid position (cell centre) of a token with the given progress.
pub fn token_pos(player: Player, token: usize, progress: i8) -> (f32, f32) {
    if progress == YARD {
        player.yard_spot(token)
    } else if progress == HOME {
        player.home_spot()
    } else if progress > LAST_TRACK {
        let (c, r) = player.home_column()[(progress - LAST_TRACK - 1) as usize];
        (c as f32, r as f32)
    } else {
        let (c, r) = TRACK[track_cell(player, progress).unwrap()];
        (c as f32, r as f32)
    }
}

/// How long one hop of a token takes on screen, in milliseconds. The client
/// animates with these and the server paces bot turns by them.
pub const HOP_MS: u64 = 120;
/// The dice tumbling and settling on its value.
pub const ROLL_MS: u64 = 650;
/// A captured token flying back to its yard.
pub const FLY_MS: u64 = 450;
/// The pop of a token arriving home.
pub const POP_MS: u64 = 250;

/// Waypoints (grid positions) a token passes through when it goes from
/// progress `from` to `to`, one per cell, excluding where it starts. Leaving
/// the yard and being sent back to it are a single jump.
pub fn path(player: Player, token: usize, from: i8, to: i8) -> Vec<(f32, f32)> {
    if from == YARD || to <= from {
        return vec![token_pos(player, token, to)];
    }
    ((from + 1)..=to)
        .map(|p| token_pos(player, token, p))
        .collect()
}

/// Time the client needs to animate these events, in milliseconds.
pub fn animation_ms(events: &[Event]) -> u64 {
    events
        .iter()
        .map(|e| match *e {
            Event::Moved { from, to, .. } => {
                let hops = if from == YARD || to <= from {
                    1
                } else {
                    (to - from) as u64
                };
                hops * HOP_MS + if to == HOME { POP_MS } else { 0 }
            }
            Event::Captured { .. } => FLY_MS,
            Event::Rolled { .. } => ROLL_MS,
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_is_a_connected_loop_with_diagonal_corner_steps() {
        for i in 0..TRACK.len() {
            let (a, b) = (TRACK[i], TRACK[(i + 1) % TRACK.len()]);
            let d = (a.0 - b.0).abs().max((a.1 - b.1).abs());
            assert_eq!(d, 1, "cells {i} and {} are not adjacent", (i + 1) % 52);
        }
    }

    #[test]
    fn start_cells_match_yards() {
        assert_eq!(TRACK[Player::Red.start_index()], (1, 6));
        assert_eq!(TRACK[Player::Green.start_index()], (8, 1));
        assert_eq!(TRACK[Player::Yellow.start_index()], (13, 8));
        assert_eq!(TRACK[Player::Blue.start_index()], (6, 13));
    }

    #[test]
    fn path_walks_one_cell_at_a_time() {
        for player in Player::ALL {
            for from in -1..HOME {
                for to in from + 1..=HOME.min(from.max(0) + 6) {
                    let steps = path(player, 0, from, to);
                    assert_eq!(
                        steps.len(),
                        if from == YARD {
                            1
                        } else {
                            (to - from) as usize
                        }
                    );
                    assert_eq!(*steps.last().unwrap(), token_pos(player, 0, to));
                    let mut prev = token_pos(player, 0, from);
                    if from != YARD {
                        for &(c, r) in &steps {
                            let d = ((c - prev.0).powi(2) + (r - prev.1).powi(2)).sqrt();
                            assert!(d <= 1.5, "{player:?} {from}->{to} jumps {d} to {c},{r}");
                            prev = (c, r);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn captured_tokens_jump_back_to_the_yard() {
        let steps = path(Player::Green, 2, 17, YARD);
        assert_eq!(steps, vec![Player::Green.yard_spot(2)]);
    }

    #[test]
    fn animation_time_adds_up() {
        let moved = Event::Moved {
            seat: 0,
            token: 0,
            from: 3,
            to: 7,
        };
        assert_eq!(animation_ms(&[moved]), 4 * HOP_MS);
        let home = Event::Moved {
            seat: 0,
            token: 0,
            from: 54,
            to: HOME,
        };
        assert_eq!(animation_ms(&[home]), 2 * HOP_MS + POP_MS);
        let out = Event::Moved {
            seat: 0,
            token: 0,
            from: YARD,
            to: 0,
        };
        let cap = Event::Captured { seat: 1, token: 0 };
        assert_eq!(
            animation_ms(&[out, cap, Event::ExtraTurn { seat: 0 }]),
            HOP_MS + FLY_MS
        );
        let rolled = Event::Rolled { seat: 0, value: 4 };
        assert_eq!(animation_ms(&[rolled]), ROLL_MS);
    }
}
