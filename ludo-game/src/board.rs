//! Static Ludo board layout on a 15x15 grid. No game logic yet.

use bevy::prelude::*;

const GRID: i32 = 15;
const CELL: f32 = 40.0;
const GAP: f32 = 2.0;

pub struct BoardPlugin;

impl Plugin for BoardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_camera, spawn_board, spawn_marker))
            .add_systems(Update, spin_marker);
    }
}

#[derive(Clone, Copy)]
enum Player {
    Red,
    Green,
    Yellow,
    Blue,
}

impl Player {
    const ALL: [Player; 4] = [Player::Red, Player::Green, Player::Yellow, Player::Blue];

    fn color(self) -> Color {
        match self {
            Player::Red => Color::srgb(0.85, 0.20, 0.20),
            Player::Green => Color::srgb(0.20, 0.70, 0.30),
            Player::Yellow => Color::srgb(0.95, 0.80, 0.20),
            Player::Blue => Color::srgb(0.20, 0.40, 0.85),
        }
    }

    /// Top-left grid cell (col, row) of this player's 6x6 yard.
    fn yard_origin(self) -> (i32, i32) {
        match self {
            Player::Red => (0, 0),
            Player::Green => (9, 0),
            Player::Yellow => (9, 9),
            Player::Blue => (0, 9),
        }
    }

    /// Cell where this player's tokens enter the track.
    fn start_cell(self) -> (i32, i32) {
        match self {
            Player::Red => (1, 6),
            Player::Green => (8, 1),
            Player::Yellow => (13, 8),
            Player::Blue => (6, 13),
        }
    }

    /// The 5 home-column cells leading to the centre.
    fn home_column(self) -> [(i32, i32); 5] {
        match self {
            Player::Red => [(1, 7), (2, 7), (3, 7), (4, 7), (5, 7)],
            Player::Green => [(7, 1), (7, 2), (7, 3), (7, 4), (7, 5)],
            Player::Yellow => [(13, 7), (12, 7), (11, 7), (10, 7), (9, 7)],
            Player::Blue => [(7, 13), (7, 12), (7, 11), (7, 10), (7, 9)],
        }
    }
}

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        // Scale to fit the whole board with a small margin, at any window size.
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::AutoMin {
                min_width: GRID as f32 * CELL + 40.0,
                min_height: GRID as f32 * CELL + 40.0,
            },
            ..OrthographicProjection::default_2d()
        }),
    ));
}

/// World position of the centre of a grid cell (row 0 is the top).
fn cell_pos(col: f32, row: f32) -> Vec2 {
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

    // Board backdrop.
    commands.spawn((
        Sprite::from_color(Color::srgb(0.2, 0.2, 0.24), Vec2::splat(GRID as f32 * CELL + 16.0)),
        Transform::from_xyz(0.0, 0.0, -1.0),
    ));

    // Cross-shaped track: any cell in the middle 3 rows or columns, outside the centre.
    for row in 0..GRID {
        for col in 0..GRID {
            let in_cross = (6..=8).contains(&row) || (6..=8).contains(&col);
            let in_centre = (6..=8).contains(&row) && (6..=8).contains(&col);
            if in_cross && !in_centre {
                tile(&mut commands, col as f32, row as f32, 1.0, 0.0, white);
            }
        }
    }

    for player in Player::ALL {
        let color = player.color();

        // Yard: coloured 6x6 block with a white 4x4 inset and four token spots.
        let (yc, yr) = player.yard_origin();
        let c = yc as f32 + 2.5;
        let r = yr as f32 + 2.5;
        tile(&mut commands, c, r, 6.0, 0.0, color);
        tile(&mut commands, c, r, 4.0, 0.1, white);
        for (dc, dr) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            tile(&mut commands, c + dc, r + dr, 1.0, 0.2, color);
        }

        // Start cell and home column.
        let (sc, sr) = player.start_cell();
        tile(&mut commands, sc as f32, sr as f32, 1.0, 0.1, color);
        for (hc, hr) in player.home_column() {
            tile(&mut commands, hc as f32, hr as f32, 1.0, 0.1, color);
        }
    }

    // Centre: four 1.5x1.5 quadrants, matching the colour of the yard in that corner.
    for (c, r, player) in [
        (6.25, 6.25, Player::Red),
        (7.75, 6.25, Player::Green),
        (7.75, 7.75, Player::Yellow),
        (6.25, 7.75, Player::Blue),
    ] {
        tile(&mut commands, c, r, 1.5, 0.1, player.color());
    }
}

/// Proof-of-life: a spinning magenta square above the board, to show the game loop is running.
#[derive(Component)]
struct Marker;

fn spawn_marker(mut commands: Commands) {
    commands.spawn((
        Marker,
        Sprite::from_color(Color::srgb(1.0, 0.0, 1.0), Vec2::splat(CELL * 0.6)),
        Transform::from_xyz(0.0, 0.0, 10.0),
    ));
}

fn spin_marker(time: Res<Time>, mut q: Query<&mut Transform, With<Marker>>) {
    for mut t in &mut q {
        t.rotate_z(time.delta_secs() * 2.0);
    }
}
