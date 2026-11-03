//! Bevy systems: pump UI/network input, draw tokens, handle clicks.

use crate::{
    board::{CELL, cell_pos, player_color},
    net, paint,
    session::Session,
};
use bevy::{asset::RenderAssetUsages, prelude::*, sprite::Anchor, window::PrimaryWindow};
use ludo_core::{
    Event, Game, HOME, TOKENS, YARD,
    board::{FLY_MS, HOP_MS, POP_MS, ROLL_MS, path, token_pos},
};
use std::collections::{HashMap, VecDeque};

pub struct PlayPlugin;

impl Plugin for PlayPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Session::new())
            .add_systems(Startup, setup_assets)
            .add_systems(
                Update,
                (
                    pump,
                    sync_tokens,
                    handle_clicks,
                    animate_tokens,
                    update_ghosts,
                )
                    .chain(),
            );
    }
}

#[derive(Component)]
struct Token {
    seat: usize,
    idx: usize,
}

/// Where a token rests once its animation is done.
#[derive(Component)]
struct Target(Vec2);

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// One cell forward, with a small jump.
    Hop,
    /// Back to the yard after a capture, in a high arc.
    Fly,
    /// Arriving home: stay put and swell.
    Pop,
}

/// One leg of a token's journey.
#[derive(Clone, Copy)]
struct Step {
    to: Vec2,
    kind: Kind,
    /// Seconds to hold still before starting.
    wait: f32,
}

impl Step {
    fn secs(&self) -> f32 {
        let ms = match self.kind {
            Kind::Hop => HOP_MS,
            Kind::Fly => FLY_MS,
            Kind::Pop => POP_MS,
        };
        ms as f32 / 1000.0
    }

    fn arc(&self) -> f32 {
        match self.kind {
            Kind::Hop => CELL * 0.35,
            Kind::Fly => CELL * 1.6,
            Kind::Pop => 0.0,
        }
    }
}

struct Active {
    step: Step,
    from: Vec2,
    /// Seconds since the step began; negative while waiting.
    t: f32,
}

/// The steps a token still has to play, in order.
#[derive(Component, Default)]
struct Motion {
    queue: VecDeque<Step>,
    active: Option<Active>,
}

/// Soft shadow under a token; it stays on the ground while the token jumps.
#[derive(Component)]
struct Shadow;

/// Queue length above which animations speed up so they never lag far behind.
const BACKLOG: usize = 8;

/// Highlight ring shown behind a token that can be moved.
#[derive(Component)]
struct Ring;

/// The pawn picture of a token. A token's entity sits on the centre of its cell;
/// the pawn stands on the ground a little below that.
#[derive(Component)]
struct PawnSprite;

/// A translucent pawn on the cell a movable token would land on.
#[derive(Component)]
struct Ghost {
    seat: usize,
    idx: usize,
}

/// Where the foot of a pawn rests, relative to the middle of its cell.
const GROUND_Y: f32 = -CELL * 0.3;
/// How large a pawn is drawn.
const PAWN_SIZE: Vec2 = Vec2::new(CELL * 0.72, CELL * 0.96);

#[derive(Resource)]
struct TokenAssets {
    ring: Handle<Mesh>,
    shadow_mesh: Handle<Mesh>,
    shadow: Handle<ColorMaterial>,
    glow: Handle<ColorMaterial>,
    pawns: [Handle<Image>; 4],
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<ColorMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let pawns = ludo_core::Player::ALL.map(|p| {
        let c = player_color(p).to_srgba();
        images.add(Image::new(
            bevy::render::render_resource::Extent3d {
                width: paint::PAWN_W as u32,
                height: paint::PAWN_H as u32,
                depth_or_array_layers: 1,
            },
            bevy::render::render_resource::TextureDimension::D2,
            paint::pawn([c.red, c.green, c.blue]),
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        ))
    });
    commands.insert_resource(TokenAssets {
        ring: meshes.add(Circle::new(CELL * 0.46)),
        shadow_mesh: meshes.add(Circle::new(CELL * 0.36)),
        shadow: mats.add(Color::srgba(0.0, 0.0, 0.0, 0.35)),
        glow: mats.add(Color::srgb(1.0, 1.0, 1.0)),
        pawns,
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
    mut session: ResMut<Session>,
    assets: Res<TokenAssets>,
    mut seen: Local<(u32, u32)>,
    existing: Query<Entity, Or<(With<Token>, With<Ghost>)>>,
    mut targets: Query<(&Token, &mut Target, &mut Motion, &mut Visibility)>,
) {
    if *seen == (session.generation, session.rev) {
        return;
    }
    let new_game = seen.0 != session.generation;
    *seen = (session.generation, session.rev);
    // Taking the events is not a change anyone else needs to hear about.
    let events = std::mem::take(&mut session.bypass_change_detection().pending);

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
                        Motion::default(),
                        Transform::from_xyz(p.x, p.y, 5.0),
                        Visibility::default(),
                    ))
                    .with_children(|t| {
                        t.spawn((
                            Shadow,
                            Mesh2d(assets.shadow_mesh.clone()),
                            MeshMaterial2d(assets.shadow.clone()),
                            Transform::from_xyz(0.0, GROUND_Y, -0.2).with_scale(SHADOW_SCALE),
                        ));
                        t.spawn((
                            Ring,
                            Mesh2d(assets.ring.clone()),
                            MeshMaterial2d(assets.glow.clone()),
                            Transform::from_xyz(0.0, GROUND_Y, -0.1)
                                .with_scale(Vec3::new(1.0, 0.5, 1.0)),
                            Visibility::Hidden,
                        ));
                        t.spawn((
                            PawnSprite,
                            Sprite {
                                image: assets.pawns[player.index()].clone(),
                                custom_size: Some(PAWN_SIZE),
                                ..default()
                            },
                            Anchor::BOTTOM_CENTER,
                            Transform::from_xyz(0.0, GROUND_Y, 0.1),
                        ));
                    });
                commands.spawn((
                    Ghost { seat, idx },
                    Sprite {
                        image: assets.pawns[player.index()].clone(),
                        custom_size: Some(PAWN_SIZE),
                        color: Color::srgba(1.0, 1.0, 1.0, 0.4),
                        ..default()
                    },
                    Anchor::BOTTOM_CENTER,
                    Transform::from_xyz(0.0, 0.0, 4.5),
                    Visibility::Hidden,
                ));
            }
        }
    } else {
        queue_events(&events, game, &positions, &mut targets);
        for (token, mut target, _, mut visibility) in &mut targets {
            target.0 = positions[token.seat][token.idx];
            // Players who left take their tokens with them.
            *visibility = if game.left[token.seat] {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
    }
    // Lock input from this very frame, before `animate_tokens` has run.
    if targets
        .iter()
        .any(|(_, _, motion, _)| !motion.queue.is_empty())
    {
        session.animating = true;
    }
}

/// Turn the game events of one update into steps on the tokens they concern.
fn queue_events(
    events: &[Event],
    game: &Game,
    positions: &[Vec<Vec2>],
    tokens: &mut Query<(&Token, &mut Target, &mut Motion, &mut Visibility)>,
) {
    // Seconds into this batch at which the latest move finishes; a capture
    // waits for the capturing token to land.
    // A roll in the same batch plays out before anything moves.
    let mut clock = if events.iter().any(|e| matches!(e, Event::Rolled { .. })) {
        ROLL_MS as f32 / 1000.0
    } else {
        0.0
    };
    for event in events {
        match *event {
            Event::Moved {
                seat,
                token,
                from,
                to,
            } => {
                let Some(&player) = game.seats.get(seat) else {
                    continue;
                };
                let mut steps: Vec<Step> = path(player, token, from, to)
                    .into_iter()
                    .map(|(c, r)| Step {
                        to: cell_pos(c, r),
                        kind: Kind::Hop,
                        wait: 0.0,
                    })
                    .collect();
                if to == HOME
                    && let Some(last) = steps.last().copied()
                {
                    steps.push(Step {
                        kind: Kind::Pop,
                        ..last
                    });
                }
                if let Some(first) = steps.first_mut() {
                    first.wait = clock;
                }
                clock += steps.iter().map(Step::secs).sum::<f32>();
                if let Some((_, _, mut motion, _)) = tokens
                    .iter_mut()
                    .find(|(t, ..)| t.seat == seat && t.idx == token)
                {
                    motion.queue.extend(steps);
                }
            }
            Event::Captured { seat, token } => {
                if let Some((_, _, mut motion, _)) = tokens
                    .iter_mut()
                    .find(|(t, ..)| t.seat == seat && t.idx == token)
                {
                    motion.queue.push_back(Step {
                        to: positions[seat][token],
                        kind: Kind::Fly,
                        wait: clock,
                    });
                }
            }
            _ => {}
        }
    }
}

/// A flattened circle under the foot of the pawn.
const SHADOW_SCALE: Vec3 = Vec3::new(1.25, 0.5, 1.0);

fn smooth(u: f32) -> f32 {
    u * u * (3.0 - 2.0 * u)
}

fn animate_tokens(
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut tokens: Query<(&Token, &Target, &mut Motion, &mut Transform, &Children)>,
    mut rings: Query<
        (&mut Visibility, &mut Transform),
        (
            With<Ring>,
            Without<Token>,
            Without<Shadow>,
            Without<PawnSprite>,
        ),
    >,
    mut shadows: Query<
        &mut Transform,
        (
            With<Shadow>,
            Without<Token>,
            Without<Ring>,
            Without<PawnSprite>,
        ),
    >,
    mut pawns: Query<
        &mut Transform,
        (
            With<PawnSprite>,
            Without<Token>,
            Without<Ring>,
            Without<Shadow>,
        ),
    >,
) {
    let backlog: usize = tokens.iter().map(|(_, _, m, ..)| m.queue.len()).sum();
    let speed = 1.0 + backlog.saturating_sub(BACKLOG) as f32 / BACKLOG as f32;
    let dt = time.delta_secs() * speed;
    let ease = 1.0 - (-14.0 * time.delta_secs()).exp();
    let pulse = 1.0 + 0.12 * (time.elapsed_secs() * 6.0).sin();
    let clickable = session.clickable();
    let mut busy = false;

    for (token, target, mut motion, mut tf, children) in &mut tokens {
        if motion.active.is_none()
            && let Some(step) = motion.queue.pop_front()
        {
            let from = tf.translation.truncate();
            motion.active = Some(Active {
                step,
                from,
                t: -step.wait,
            });
        }

        let mut pos = tf.translation.truncate();
        let (mut lift, mut scale, mut flying) = (0.0, Vec3::ONE, false);
        let mut done = false;
        if let Some(active) = &mut motion.active {
            busy = true;
            active.t += dt;
            let secs = active.step.secs();
            if active.t >= secs {
                pos = active.step.to;
                done = true;
            } else if active.t < 0.0 {
                pos = active.from;
            } else {
                let u = active.t / secs;
                let arc = (u * std::f32::consts::PI).sin();
                flying = true;
                match active.step.kind {
                    Kind::Hop => {
                        pos = active.from.lerp(active.step.to, smooth(u));
                        lift = active.step.arc() * arc;
                        scale = Vec3::new(1.0 - 0.06 * arc, 1.0 + 0.12 * arc, 1.0);
                        if u > 0.8 {
                            // Squash on landing.
                            let k = ((u - 0.8) / 0.2 * std::f32::consts::PI).sin() * 0.12;
                            scale = Vec3::new(1.0 + k, 1.0 - k, 1.0);
                        }
                    }
                    Kind::Fly => {
                        pos = active.from.lerp(active.step.to, 1.0 - (1.0 - u).powi(2));
                        lift = active.step.arc() * arc;
                        scale = Vec3::splat(1.0 + 0.2 * arc);
                    }
                    Kind::Pop => {
                        pos = active.step.to;
                        scale = Vec3::splat(1.0 + 0.35 * arc);
                    }
                }
            }
        } else {
            pos += (target.0 - pos) * ease;
        }
        if done {
            motion.active = None;
        }
        busy |= !motion.queue.is_empty();

        // Tokens lower on the screen are in front; a jumping one is in front of all.
        let z = if flying { 6.0 } else { 5.0 - pos.y * 0.0005 };
        tf.translation = Vec3::new(pos.x, pos.y + lift, z);
        tf.scale = scale;

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
                ring_tf.scale = Vec3::new(pulse, 0.5 * pulse, 1.0);
            }
            if let Ok(mut pawn_tf) = pawns.get_mut(*child) {
                // A movable pawn bobs to catch the eye.
                let bob = if active {
                    (time.elapsed_secs() * 8.0).sin().abs() * CELL * 0.06
                } else {
                    0.0
                };
                pawn_tf.translation.y = GROUND_Y + bob;
            }
            if let Ok(mut shadow_tf) = shadows.get_mut(*child) {
                // Stay on the ground (undo the lift) and shrink as the token rises.
                let shrink = (1.0 - lift / (CELL * 3.0)).max(0.5);
                shadow_tf.translation.y = GROUND_Y - lift;
                shadow_tf.scale = SHADOW_SCALE * Vec3::new(shrink, shrink, 1.0);
            }
        }
    }

    if session.animating != busy {
        session.animating = busy;
    }
}

/// Show where each movable token would land with the rolled value.
fn update_ghosts(
    time: Res<Time>,
    session: Res<Session>,
    mut ghosts: Query<(&Ghost, &mut Transform, &mut Visibility, &mut Sprite)>,
) {
    let clickable = session.clickable();
    let game = session.game.as_ref();
    let alpha = 0.35 + 0.15 * (time.elapsed_secs() * 6.0).sin();
    for (ghost, mut tf, mut vis, mut sprite) in &mut ghosts {
        let landing = clickable
            .as_ref()
            .filter(|(seat, idxs)| *seat == ghost.seat && idxs.contains(&ghost.idx))
            .zip(game)
            .and_then(|(_, g)| Some((g, g.dice?)));
        let Some((game, dice)) = landing else {
            *vis = Visibility::Hidden;
            continue;
        };
        let from = game.tokens[ghost.seat][ghost.idx];
        let to = if from == YARD { 0 } else { from + dice as i8 };
        let (c, r) = token_pos(game.seats[ghost.seat], ghost.idx, to);
        let at = cell_pos(c, r);
        tf.translation = Vec3::new(at.x, at.y + GROUND_Y, 4.5 - at.y * 0.0005);
        sprite.color = Color::srgba(1.0, 1.0, 1.0, alpha);
        *vis = Visibility::Inherited;
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
        // Aim at the middle of the standing pawn, not of its cell.
        .map(|(t, target)| (t.idx, (target.0 + Vec2::Y * CELL * 0.13).distance(world)))
        .filter(|(_, d)| *d < CELL * 0.5)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((idx, _)) = hit {
        session.play(idx);
    }
}
