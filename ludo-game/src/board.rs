//! The Ludo board, laid out from the geometry in `ludo-core`.

use crate::paint;
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use ludo_core::{Player, board::GRID};

pub const CELL: f32 = 40.0;
/// World size of the board square plus a small margin.
pub const EXTENT: f32 = GRID as f32 * CELL + 40.0;

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

/// The board is one sprite with a texture painted at startup (see `paint`).
fn spawn_board(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let colors = Player::ALL.map(|p| {
        let c = player_color(p).to_srgba();
        [c.red, c.green, c.blue]
    });
    let side = paint::SIDE as u32;
    let image = Image::new(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        paint::render(colors),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    commands.spawn((
        Sprite {
            image: images.add(image),
            custom_size: Some(Vec2::splat(EXTENT)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, -1.0),
    ));
}
