//! Visual effects: bursts of sparks where a token is captured or gets home,
//! confetti for the winner, and a short buzz on phones.

use crate::{
    board::{CELL, player_color},
    paint,
    session::{Screen, Session},
};
use bevy::{
    asset::RenderAssetUsages,
    color::Alpha,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

pub struct FxPlugin;

impl Plugin for FxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_fx)
            .add_systems(Update, (update_particles, winner_confetti, update_confetti));
    }
}

#[derive(Resource)]
pub struct FxAssets {
    disc: Handle<Image>,
    spark: Handle<Image>,
}

fn setup_fx(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut make = |data: Vec<u8>| {
        images.add(Image::new(
            Extent3d {
                width: paint::FX_SIZE as u32,
                height: paint::FX_SIZE as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        ))
    };
    commands.insert_resource(FxAssets {
        disc: make(paint::soft_disc()),
        spark: make(paint::spark()),
    });
}

fn rand() -> f32 {
    js_sys::Math::random() as f32
}

fn between(lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * rand()
}

/// Effects are on, and the player has not asked for less motion.
pub fn enabled(session: &Session) -> bool {
    session.settings.effects && !session.settings.reduced_motion
}

// ---- world particles --------------------------------------------------------

#[derive(Component)]
struct Particle {
    vel: Vec2,
    /// Added to the vertical speed every second (negative pulls down).
    gravity: f32,
    age: f32,
    life: f32,
    spin: f32,
    /// Extra size per second, as a fraction of the starting size.
    grow: f32,
    size: f32,
    alpha: f32,
}

#[derive(Clone, Copy)]
pub enum Burst {
    /// A token knocked back to its yard.
    Capture,
    /// A token reaching home.
    Home,
}

const GOLD: Color = Color::srgb(1.0, 0.82, 0.3);

fn particle(commands: &mut Commands, image: &Handle<Image>, at: Vec2, color: Color, p: Particle) {
    commands.spawn((
        Sprite {
            image: image.clone(),
            color: color.with_alpha(p.alpha),
            custom_size: Some(Vec2::splat(p.size)),
            ..default()
        },
        Transform::from_xyz(at.x, at.y, 7.0)
            .with_rotation(Quat::from_rotation_z(rand() * std::f32::consts::TAU)),
        p,
    ));
}

/// Spawn a burst of particles at a world position.
pub fn burst(commands: &mut Commands, assets: &FxAssets, at: Vec2, color: Color, kind: Burst) {
    match kind {
        Burst::Capture => {
            // A flash, some dust and a spray of sparks in the victim's colour.
            particle(
                commands,
                &assets.disc,
                at,
                Color::WHITE,
                Particle {
                    vel: Vec2::ZERO,
                    gravity: 0.0,
                    age: 0.0,
                    life: 0.35,
                    spin: 0.0,
                    grow: 4.0,
                    size: CELL * 0.5,
                    alpha: 0.8,
                },
            );
            for _ in 0..6 {
                let dir = Vec2::from_angle(rand() * std::f32::consts::TAU);
                particle(
                    commands,
                    &assets.disc,
                    at,
                    Color::srgb(0.75, 0.68, 0.58),
                    Particle {
                        vel: dir * between(20.0, 60.0),
                        gravity: 0.0,
                        age: 0.0,
                        life: between(0.5, 0.8),
                        spin: 0.0,
                        grow: 1.5,
                        size: CELL * 0.4,
                        alpha: 0.5,
                    },
                );
            }
            for i in 0..12 {
                let dir = Vec2::from_angle(rand() * std::f32::consts::TAU);
                let tint = if i % 3 == 0 { Color::WHITE } else { color };
                particle(
                    commands,
                    &assets.spark,
                    at,
                    tint,
                    Particle {
                        vel: dir * between(80.0, 180.0) + Vec2::Y * 40.0,
                        gravity: -260.0,
                        age: 0.0,
                        life: between(0.5, 0.9),
                        spin: between(-8.0, 8.0),
                        grow: 0.0,
                        size: between(CELL * 0.25, CELL * 0.45),
                        alpha: 1.0,
                    },
                );
            }
        }
        Burst::Home => {
            particle(
                commands,
                &assets.disc,
                at,
                GOLD,
                Particle {
                    vel: Vec2::ZERO,
                    gravity: 0.0,
                    age: 0.0,
                    life: 0.5,
                    spin: 0.0,
                    grow: 5.0,
                    size: CELL * 0.5,
                    alpha: 0.7,
                },
            );
            for i in 0..18 {
                let angle = between(0.5, std::f32::consts::PI - 0.5);
                let tint = match i % 3 {
                    0 => GOLD,
                    1 => color,
                    _ => Color::WHITE,
                };
                particle(
                    commands,
                    &assets.spark,
                    at,
                    tint,
                    Particle {
                        vel: Vec2::from_angle(angle) * between(100.0, 230.0),
                        gravity: -320.0,
                        age: 0.0,
                        life: between(0.8, 1.3),
                        spin: between(-6.0, 6.0),
                        grow: 0.0,
                        size: between(CELL * 0.25, CELL * 0.5),
                        alpha: 1.0,
                    },
                );
            }
        }
    }
}

fn update_particles(
    mut commands: Commands,
    time: Res<Time>,
    mut particles: Query<(Entity, &mut Particle, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    for (entity, mut p, mut tf, mut sprite) in &mut particles {
        p.age += dt;
        if p.age >= p.life {
            commands.entity(entity).despawn();
            continue;
        }
        let t = p.age / p.life;
        let vel = p.vel;
        tf.translation += (vel * dt).extend(0.0);
        p.vel.y += p.gravity * dt;
        tf.rotate_z(p.spin * dt);
        sprite.custom_size = Some(Vec2::splat(p.size * (1.0 + p.grow * p.age)));
        let a = p.alpha * (1.0 - t).powf(1.2);
        sprite.color = sprite.color.with_alpha(a);
    }
}

// ---- haptics ----------------------------------------------------------------

/// A short buzz on phones that support it; ignored elsewhere.
pub fn vibrate(session: &Session, ms: u32) {
    if !session.settings.effects {
        return;
    }
    if let Some(window) = web_sys::window() {
        let _ = window.navigator().vibrate_with_duration(ms);
    }
}

// ---- winner confetti --------------------------------------------------------

/// A piece of confetti falling over everything, as a UI node so it shows above
/// the game-over panel. Positions are percentages of the screen.
#[derive(Component)]
struct Confetti {
    pos: Vec2,
    vel: Vec2,
    phase: f32,
    spin: f32,
}

#[derive(Default)]
struct Rain {
    /// Seconds of confetti still to fall; `None` until this win has started.
    left: Option<f32>,
    /// Pieces owed to the screen, as a fraction.
    owed: f32,
}

const RAIN_SECS: f32 = 3.5;
const PIECES_PER_SEC: f32 = 45.0;

fn winner_confetti(
    mut commands: Commands,
    time: Res<Time>,
    session: Res<Session>,
    mut rain: Local<Rain>,
) {
    let winner = session
        .game
        .as_ref()
        .and_then(|g| g.winner.map(|w| g.seats[w]));
    let showing = session.screen == Screen::Game
        && !session.animating
        && !session.dice_busy
        && winner.is_some();
    let Some(player) = winner.filter(|_| showing) else {
        if winner.is_none() {
            rain.left = None;
        }
        return;
    };
    if rain.left.is_none() {
        rain.left = Some(RAIN_SECS);
        rain.owed = 0.0;
        let mine = !session.in_room || session.game.as_ref().and_then(|g| g.winner) == session.you;
        if mine {
            vibrate(&session, 120);
        }
    }
    let dt = time.delta_secs();
    let Some(left) = rain.left.as_mut().filter(|l| **l > 0.0) else {
        return;
    };
    *left -= dt;
    if !enabled(&session) {
        return;
    }
    rain.owed += PIECES_PER_SEC * dt;
    let palette = [
        player_color(player),
        player_color(player),
        GOLD,
        Color::WHITE,
        player_color(ludo_core::Player::ALL[(rand() * 4.0) as usize % 4]),
    ];
    while rain.owed >= 1.0 {
        rain.owed -= 1.0;
        let color = palette[(rand() * palette.len() as f32) as usize % palette.len()];
        let pos = Vec2::new(between(0.0, 100.0), -4.0);
        commands.spawn((
            Confetti {
                pos,
                vel: Vec2::new(0.0, between(28.0, 55.0)),
                phase: rand() * std::f32::consts::TAU,
                spin: between(-5.0, 5.0),
            },
            Node {
                position_type: PositionType::Absolute,
                left: percent(pos.x),
                top: percent(pos.y),
                width: vmin(between(1.2, 2.2)),
                height: vmin(between(0.7, 1.3)),
                ..default()
            },
            BackgroundColor(color),
            UiTransform::default(),
            GlobalZIndex(12),
        ));
    }
}

fn update_confetti(
    mut commands: Commands,
    time: Res<Time>,
    mut pieces: Query<(Entity, &mut Confetti, &mut Node, &mut UiTransform)>,
) {
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    for (entity, mut c, mut node, mut tf) in &mut pieces {
        let vel = c.vel;
        c.pos += vel * dt;
        if c.pos.y > 104.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let sway = (t * 2.5 + c.phase).sin() * 5.0;
        node.left = percent(c.pos.x + sway);
        node.top = percent(c.pos.y);
        tf.rotation = Rot2::radians(t * c.spin + c.phase);
    }
}
