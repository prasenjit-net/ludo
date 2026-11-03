//! The static Ludo board, drawn from the geometry in `ludo-core`.

use bevy::prelude::*;
use ludo_core::{
    Player,
    board::{GRID, TRACK},
};

pub const CELL: f32 = 40.0;
/// World size of the board square plus a small margin.
pub const EXTENT: f32 = GRID as f32 * CELL + 40.0;
const GAP: f32 = 2.0;

pub struct BoardPlugin;

impl Plugin for BoardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_camera, spawn_board));
    }
}

pub fn player_color(player: Player) -> Color {
    match player {
        Player::Red => Color::srgb(0.85, 0.20, 0.20),
        Player::Green => Color::srgb(0.20, 0.70, 0.30),
        Player::Yellow => Color::srgb(0.95, 0.80, 0.20),
        Player::Blue => Color::srgb(0.20, 0.40, 0.85),
    }
}

fn setup_camera(mut commands: Commands) {
    // Framing is driven by `ui::fit_board`, which fits the board into its layout area.
    commands.spawn(Camera2d);
}

/// World position of the centre of a grid cell (row 0 is the top).
pub fn cell_pos(col: f32, row: f32) -> Vec2 {
    let mid = (GRID as f32 - 1.0) / 2.0;
    Vec2::new((col - mid) * CELL, (mid - row) * CELL)
}

fn tile(commands: &mut Commands, col: f32, row: f32, size: f32, z: f32, color: Color) {
    let p = cell_pos(col, row);
    commands.spawn((
        Sprite::from_color(color, Vec2::splat(size * CELL - GAP)),
        Transform::from_xyz(p.x, p.y, z),
    ));
}

fn spawn_board(mut commands: Commands) {
    let white = Color::srgb(0.96, 0.95, 0.92);
    let star = Color::srgb(0.80, 0.78, 0.72);

    commands.spawn((
        Sprite::from_color(
            Color::srgb(0.2, 0.2, 0.24),
            Vec2::splat(GRID as f32 * CELL + 16.0),
        ),
        Transform::from_xyz(0.0, 0.0, -1.0),
    ));

    // Shared track; start cells and star cells (safe squares) get a tint below.
    for (i, &(col, row)) in TRACK.iter().enumerate() {
        let safe = i % 13 == 8;
        tile(
            &mut commands,
            col as f32,
            row as f32,
            1.0,
            0.0,
            if safe { star } else { white },
        );
    }

    for player in Player::ALL {
        let color = player_color(player);

        // Yard: coloured 6x6 block with a white 4x4 inset and four token spots.
        let (yc, yr) = player.yard_origin();
        let (c, r) = (yc as f32 + 2.5, yr as f32 + 2.5);
        tile(&mut commands, c, r, 6.0, 0.0, color);
        tile(&mut commands, c, r, 4.0, 0.1, white);
        for (dc, dr) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            tile(
                &mut commands,
                c + dc,
                r + dr,
                1.0,
                0.2,
                color.with_alpha(0.45),
            );
        }

        let (sc, sr) = TRACK[player.start_index()];
        tile(&mut commands, sc as f32, sr as f32, 1.0, 0.1, color);
        for (hc, hr) in player.home_column() {
            tile(&mut commands, hc as f32, hr as f32, 1.0, 0.1, color);
        }
    }

    // Centre: four quadrants, coloured like the yard in that corner.
    for (c, r, player) in [
        (6.25, 6.25, Player::Red),
        (7.75, 6.25, Player::Green),
        (7.75, 7.75, Player::Yellow),
        (6.25, 7.75, Player::Blue),
    ] {
        tile(&mut commands, c, r, 1.5, 0.1, player_color(player));
    }
}
