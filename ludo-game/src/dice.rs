//! The die in the control panel: pips drawn with `bevy_ui`, a tumble while the
//! roll is in flight and a settle onto the rolled value.

use crate::{
    board::player_color,
    session::{Session, random_die},
};
use bevy::{ecs::spawn::SpawnIter, prelude::*};
use std::f32::consts::{PI, TAU};

/// The die can tumble for this long at most before giving up on the server.
const TUMBLE_MAX: f32 = 3.0;
/// The shortest tumble, and the settle that follows it; together `ROLL_MS`.
const TUMBLE_MIN: f32 = 0.5;
const SETTLE: f32 = 0.15;
/// Seconds between faces while tumbling.
const FLIP: f32 = 0.07;

const FACE: Color = Color::srgb(0.97, 0.96, 0.93);
const PIP: Color = Color::srgb(0.1, 0.1, 0.12);
const RIM: Color = Color::srgb(0.63, 0.62, 0.66);

pub struct DicePlugin;

impl Plugin for DicePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DieState>()
            .add_systems(Update, animate_die);
    }
}

#[derive(Component)]
pub struct DieFace;

/// One of the nine pip positions, numbered row by row.
#[derive(Component)]
struct Pip(usize);

/// Which of the nine positions hold a pip for each face.
const PIPS: [&[usize]; 6] = [
    &[4],
    &[0, 8],
    &[0, 4, 8],
    &[0, 2, 6, 8],
    &[0, 2, 4, 6, 8],
    &[0, 2, 3, 5, 6, 8],
];

/// The die as a UI node, `size` wide and tall.
pub fn die(size: Val) -> impl Bundle {
    let pips: Vec<_> = (0..9)
        .map(|i| {
            let (col, row) = ((i % 3) as f32, (i / 3) as f32);
            (
                Pip(i),
                Node {
                    position_type: PositionType::Absolute,
                    left: percent(col * 28.0 + 9.0),
                    top: percent(row * 28.0 + 9.0),
                    width: percent(18),
                    height: percent(18),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(PIP),
                Visibility::Hidden,
            )
        })
        .collect();
    (
        DieFace,
        Node {
            width: size,
            height: size,
            border: UiRect::all(percent(4)),
            border_radius: BorderRadius::all(percent(20)),
            ..default()
        },
        BackgroundColor(FACE),
        BorderColor::all(RIM),
        Children::spawn(SpawnIter(pips.into_iter())),
    )
}

enum Stage {
    Idle,
    /// Tumbling; `settle` is the value to land on once the server has answered.
    Tumble {
        t: f32,
        flip: f32,
        settle: Option<u8>,
    },
    Settle {
        t: f32,
    },
}

#[derive(Resource)]
struct DieState {
    stage: Stage,
    /// Face currently drawn (0 = blank), to avoid redoing the pips every frame.
    shown: u8,
    seen_rolls: u32,
    /// Rim colour currently drawn, as the roller's seat.
    rim: Option<usize>,
}

impl Default for DieState {
    fn default() -> Self {
        DieState {
            stage: Stage::Idle,
            shown: 0,
            seen_rolls: 0,
            rim: None,
        }
    }
}

fn animate_die(
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut state: ResMut<DieState>,
    mut dice: Query<(&mut UiTransform, &mut BorderColor), With<DieFace>>,
    mut pips: Query<(&Pip, &mut Visibility)>,
) {
    let dt = time.delta_secs();
    let state = &mut *state;
    let reduced = session.settings.reduced_motion;

    // A new roll has been seen: settle the running tumble, or tumble for it now.
    let rolled = session.roll_count != state.seen_rolls;
    state.seen_rolls = session.roll_count;
    let value = session.last_roll;
    match &mut state.stage {
        Stage::Tumble { settle, .. } if rolled => *settle = value,
        Stage::Idle | Stage::Settle { .. } if rolled => {
            state.stage = Stage::Tumble {
                t: 0.0,
                flip: 0.0,
                settle: value,
            };
        }
        Stage::Idle | Stage::Settle { .. } if session.rolling => {
            state.stage = Stage::Tumble {
                t: 0.0,
                flip: 0.0,
                settle: None,
            };
        }
        _ => {}
    }

    let mut face = state.shown;
    let mut transform = UiTransform::IDENTITY;
    match &mut state.stage {
        Stage::Idle => face = session.die().unwrap_or(0),
        Stage::Tumble { t, flip, settle } => {
            *t += dt;
            *flip -= dt;
            if *flip <= 0.0 {
                *flip = FLIP;
                // A different face each time.
                face = (face.max(1) - 1 + random_die() % 5 + 1) % 6 + 1;
            }
            if !reduced {
                transform.rotation = Rot2::radians(*t * TAU * 2.0);
                transform.translation = Val2::new(Val::ZERO, vmin(-3.0 * (*t * 16.0).sin().abs()));
            }
            let min = if reduced { 0.1 } else { TUMBLE_MIN };
            let done = settle.filter(|_| *t >= min);
            if let Some(v) = done {
                face = v;
                state.stage = Stage::Settle { t: 0.0 };
            } else if *t > TUMBLE_MAX {
                // The server never answered; fall back to the last known roll.
                session.rolling = false;
                state.stage = Stage::Idle;
            }
        }
        Stage::Settle { t } => {
            *t += dt;
            let u = (*t / SETTLE).min(1.0);
            if !reduced {
                transform.rotation = Rot2::radians(0.35 * (1.0 - u) * (1.0 - u));
                transform.scale = Vec2::splat(1.0 + 0.2 * (u * PI).sin());
            }
            if u >= 1.0 {
                state.stage = Stage::Idle;
            }
        }
    }

    let busy = !matches!(state.stage, Stage::Idle);
    if session.dice_busy != busy {
        session.dice_busy = busy;
    }

    if face != state.shown {
        state.shown = face;
        for (pip, mut vis) in &mut pips {
            let on = face != 0 && PIPS[face as usize - 1].contains(&pip.0);
            *vis = if on {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    }
    let roller = session.last_roller;
    for (mut tf, mut border) in &mut dice {
        *tf = transform;
        if roller != state.rim {
            let color = roller
                .and_then(|s| session.game.as_ref()?.seats.get(s))
                .map_or(RIM, |&p| player_color(p));
            *border = BorderColor::all(color);
        }
    }
    state.rim = roller;
}

fn vmin(v: f32) -> Val {
    Val::VMin(v)
}
