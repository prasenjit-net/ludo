//! Bevy systems: pump UI/network input, draw tokens, handle clicks.

use crate::{
    board::{CELL, cell_pos, player_color},
    net,
    session::Session,
};
use bevy::{prelude::*, window::PrimaryWindow};
use ludo_core::{Game, TOKENS, board::token_pos};
use std::collections::HashMap;

pub struct PlayPlugin;

impl Plugin for PlayPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Session::new())
            .add_systems(Startup, setup_assets)
            .add_systems(
                Update,
                (pump, sync_tokens, handle_clicks, animate_tokens).chain(),
            );
    }
}

#[derive(Component)]
struct Token {
    seat: usize,
    idx: usize,
}

#[derive(Component)]
struct Target(Vec2);

/// Highlight ring shown behind a token that can be moved.
#[derive(Component)]
struct Ring;

#[derive(Resource)]
struct TokenAssets {
    ring: Handle<Mesh>,
    outer: Handle<Mesh>,
    inner: Handle<Mesh>,
    dark: Handle<ColorMaterial>,
    glow: Handle<ColorMaterial>,
    players: [Handle<ColorMaterial>; 4],
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<ColorMaterial>>,
) {
    commands.insert_resource(TokenAssets {
        ring: meshes.add(Circle::new(CELL * 0.46)),
        outer: meshes.add(Circle::new(CELL * 0.36)),
        inner: meshes.add(Circle::new(CELL * 0.27)),
        dark: mats.add(Color::srgb(0.08, 0.08, 0.1)),
        glow: mats.add(Color::srgb(1.0, 1.0, 1.0)),
        players: ludo_core::Player::ALL.map(|p| mats.add(player_color(p))),
    });
}

/// Connect on start (once there is a profile), feed server messages into the
/// session and retry dropped connections. The session is only marked changed
/// when something actually happens.
fn pump(mut session: ResMut<Session>, time: Res<Time>, mut booted: Local<bool>) {
    if !*booted {
        *booted = true;
        if session.has_profile {
            session.connect();
        }
    }
    for ev in net::poll() {
        session.handle_net(ev);
    }
    if let Some(left) = session.bypass_change_detection().reconnect_in.as_mut() {
        *left -= time.delta_secs();
        if *left <= 0.0 {
            session.connect();
        }
    }
}

/// World positions for every token, spreading tokens that share a cell.
fn token_targets(game: &Game) -> Vec<Vec<Vec2>> {
    let mut base = Vec::new();
    let mut groups: HashMap<(i32, i32), usize> = HashMap::new();
    for (seat, &player) in game.seats.iter().enumerate() {
        for idx in 0..TOKENS {
            let (c, r) = token_pos(player, idx, game.tokens[seat][idx]);
            let key = ((c * 100.0) as i32, (r * 100.0) as i32);
            *groups.entry(key).or_default() += 1;
            base.push((seat, idx, key, cell_pos(c, r)));
        }
    }
    let mut seen: HashMap<(i32, i32), usize> = HashMap::new();
    let mut out = vec![vec![Vec2::ZERO; TOKENS]; game.seats.len()];
    for (seat, idx, key, pos) in base {
        let n = groups[&key];
        let k = seen.entry(key).or_default();
        let offset = if n > 1 {
            let a = *k as f32 / n as f32 * std::f32::consts::TAU + 0.6;
            Vec2::from_angle(a) * CELL * 0.2
        } else {
            Vec2::ZERO
        };
        *k += 1;
        out[seat][idx] = pos + offset;
    }
    out
}

/// Rebuild token entities for a new game; otherwise just retarget them.
fn sync_tokens(
    mut commands: Commands,
    session: Res<Session>,
    assets: Res<TokenAssets>,
    mut seen: Local<(u32, u32)>,
    existing: Query<Entity, With<Token>>,
    mut targets: Query<(&Token, &mut Target, &mut Visibility)>,
) {
    if *seen == (session.generation, session.rev) {
        return;
    }
    let new_game = seen.0 != session.generation;
    *seen = (session.generation, session.rev);

    if new_game {
        for e in &existing {
            commands.entity(e).despawn();
        }
    }
    let Some(game) = &session.game else { return };
    let positions = token_targets(game);

    if new_game {
        for (seat, &player) in game.seats.iter().enumerate() {
            for (idx, &p) in positions[seat].iter().enumerate() {
                commands
                    .spawn((
                        Token { seat, idx },
                        Target(p),
                        Transform::from_xyz(p.x, p.y, 5.0),
                        Visibility::default(),
                    ))
                    .with_children(|t| {
                        t.spawn((
                            Ring,
                            Mesh2d(assets.ring.clone()),
                            MeshMaterial2d(assets.glow.clone()),
                            Transform::from_xyz(0.0, 0.0, -0.1),
                            Visibility::Hidden,
                        ));
                        t.spawn((
                            Mesh2d(assets.outer.clone()),
                            MeshMaterial2d(assets.dark.clone()),
                        ));
                        t.spawn((
                            Mesh2d(assets.inner.clone()),
                            MeshMaterial2d(assets.players[player.index()].clone()),
                            Transform::from_xyz(0.0, 0.0, 0.1),
                        ));
                    });
            }
        }
    } else {
        for (token, mut target, mut visibility) in &mut targets {
            target.0 = positions[token.seat][token.idx];
            // Players who left take their tokens with them.
            *visibility = if game.left[token.seat] {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
    }
}

fn animate_tokens(
    time: Res<Time>,
    session: Res<Session>,
    mut tokens: Query<(&Token, &Target, &mut Transform, &Children)>,
    mut rings: Query<(&mut Visibility, &mut Transform), (With<Ring>, Without<Token>)>,
) {
    let ease = 1.0 - (-14.0 * time.delta_secs()).exp();
    let pulse = 1.0 + 0.12 * (time.elapsed_secs() * 6.0).sin();
    let clickable = session.clickable();
    for (token, target, mut tf, children) in &mut tokens {
        let cur = tf.translation.truncate();
        let next = cur + (target.0 - cur) * ease;
        tf.translation.x = next.x;
        tf.translation.y = next.y;

        let active = clickable
            .as_ref()
            .is_some_and(|(seat, idxs)| *seat == token.seat && idxs.contains(&token.idx));
        for child in children {
            if let Ok((mut vis, mut ring_tf)) = rings.get_mut(*child) {
                *vis = if active {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                ring_tf.scale = Vec3::splat(pulse);
            }
        }
    }
}

fn handle_clicks(
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    tokens: Query<(&Token, &Target)>,
    mut session: ResMut<Session>,
) {
    let screen_pos = if mouse.just_pressed(MouseButton::Left) {
        windows.single().ok().and_then(|w| w.cursor_position())
    } else {
        touches.iter_just_pressed().next().map(|t| t.position())
    };
    let Some(screen_pos) = screen_pos else { return };
    let Some((seat, movable)) = session.clickable() else {
        return;
    };
    let Ok((camera, cam_tf)) = cameras.single() else {
        return;
    };
    let Ok(world) = camera.viewport_to_world_2d(cam_tf, screen_pos) else {
        return;
    };

    let hit = tokens
        .iter()
        .filter(|(t, _)| t.seat == seat && movable.contains(&t.idx))
        .map(|(t, target)| (t.idx, target.0.distance(world)))
        .filter(|(_, d)| *d < CELL * 0.5)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((idx, _)) = hit {
        session.play(idx);
    }
}
