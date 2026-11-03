//! All user interface, built with `bevy_ui`. Sizes use `vmin` so the layout
//! scales with the screen; portrait (phone) is the primary layout, and
//! landscape moves the control panel beside the board.

use crate::{
    board::{EXTENT, player_color},
    session::{Action, CODE_LEN, Screen, Session},
};
use bevy::{
    camera::ScalingMode,
    color::Mix,
    prelude::*,
    ui::{InteractionDisabled, Pressed, UiGlobalTransform},
    ui_widgets::{Activate, Button, observe},
    window::PrimaryWindow,
};

const BG: Color = Color::srgb(0.12, 0.12, 0.14);
const PANEL: Color = Color::srgb(0.17, 0.17, 0.2);
const CHIP: Color = Color::srgb(0.12, 0.12, 0.14);
const BUTTON: Color = Color::srgb(0.27, 0.27, 0.31);
const ACCENT: Color = Color::srgb(0.95, 0.69, 0.22);
const TEXT: Color = Color::srgb(0.95, 0.94, 0.92);
const DARK: Color = Color::srgb(0.1, 0.1, 0.12);
const MUTED: Color = Color::srgb(0.63, 0.62, 0.66);
const ERROR: Color = Color::srgb(1.0, 0.55, 0.55);

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_ui).add_systems(
            Update,
            (
                responsive_layout,
                show_overlays,
                sync_fields,
                sync_slots,
                sync_roll_button,
                button_look,
                fit_board,
            ),
        );
    }
}

// ---- components -------------------------------------------------------------

#[derive(Component)]
enum Overlay {
    Screen(Screen),
    GameOver,
}

/// Parts of the in-game layout that change between portrait and landscape.
#[derive(Component, PartialEq)]
enum Part {
    Root,
    Board,
    Panel,
}

/// The layout area the board is drawn into (see `fit_board`).
#[derive(Component)]
struct BoardArea;

/// A text whose content comes from the session.
#[derive(Component)]
enum Field {
    JoinCode,
    Error,
    Room,
    Wait,
    Status,
    Event,
    Die,
    Winner,
}

/// A player chip; `lobby` chips live in the lobby, the others in the game panel.
#[derive(Component)]
struct Slot {
    lobby: bool,
    idx: usize,
}

#[derive(Component)]
struct Dot;

#[derive(Component)]
struct SlotLabel;

#[derive(Component)]
struct RollButton;

/// Resting colour of a button.
#[derive(Component)]
struct Tint(Color);

// ---- builders ---------------------------------------------------------------

fn label(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: FontSize::VMin(size),
            ..default()
        },
        TextColor(color),
    )
}

fn button(text: impl Into<String>, action: Action, primary: bool, height: f32) -> impl Bundle {
    let (tint, fg) = if primary {
        (ACCENT, DARK)
    } else {
        (BUTTON, TEXT)
    };
    (
        Button,
        Tint(tint),
        BackgroundColor(tint),
        Node {
            flex_grow: 1.0,
            flex_basis: px(0),
            height: vmin(height),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(vmin(2.0)),
            ..default()
        },
        observe(move |_: On<Activate>, mut session: ResMut<Session>| session.apply(action)),
        children![label(text, height * 0.42, fg)],
    )
}

fn row() -> Node {
    Node {
        width: percent(100),
        max_width: vmin(90),
        column_gap: vmin(2.0),
        justify_content: JustifyContent::Center,
        ..default()
    }
}

fn overlay(kind: Overlay, background: Color) -> impl Bundle {
    (
        kind,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: vmin(2.5),
            padding: UiRect::all(vmin(4.0)),
            ..default()
        },
        BackgroundColor(background),
        GlobalZIndex(10),
    )
}

fn heading(text: &str) -> impl Bundle {
    label(text.to_uppercase(), 3.2, MUTED)
}

fn slot(lobby: bool, idx: usize) -> impl Bundle {
    (
        Slot { lobby, idx },
        Node {
            align_items: AlignItems::Center,
            column_gap: vmin(1.5),
            padding: UiRect::axes(vmin(2.0), vmin(1.2)),
            border: UiRect::all(vmin(0.7)),
            border_radius: BorderRadius::all(vmin(2.0)),
            ..default()
        },
        BackgroundColor(CHIP),
        BorderColor::all(Color::NONE),
        children![
            (
                Dot,
                Node {
                    width: vmin(4.0),
                    height: vmin(4.0),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(Color::WHITE),
            ),
            (SlotLabel, label("", 3.4, TEXT)),
        ],
    )
}

fn key(digit: u8) -> impl Bundle {
    button(digit.to_string(), Action::Digit(digit), false, 10.0)
}

// ---- setup ------------------------------------------------------------------

fn spawn_ui(mut commands: Commands) {
    // In-game layout: the board area, then the control panel.
    commands.spawn((
        Part::Root,
        Node {
            width: percent(100),
            height: percent(100),
            ..default()
        },
        children![
            (
                Part::Board,
                BoardArea,
                Node {
                    aspect_ratio: Some(1.0),
                    flex_shrink: 1.0,
                    ..default()
                },
            ),
            (
                Part::Panel,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceEvenly,
                    padding: UiRect::all(vmin(3.0)),
                    ..default()
                },
                BackgroundColor(PANEL),
                children![
                    (
                        Node {
                            width: percent(100),
                            flex_wrap: FlexWrap::Wrap,
                            justify_content: JustifyContent::Center,
                            column_gap: vmin(2.0),
                            row_gap: vmin(1.5),
                            ..default()
                        },
                        children![
                            slot(false, 0),
                            slot(false, 1),
                            slot(false, 2),
                            slot(false, 3)
                        ],
                    ),
                    (
                        row(),
                        children![
                            (
                                Node {
                                    width: vmin(18.0),
                                    height: vmin(18.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    border_radius: BorderRadius::all(vmin(3.0)),
                                    ..default()
                                },
                                BackgroundColor(TEXT),
                                children![(Field::Die, label("-", 11.0, DARK))],
                            ),
                            (button("ROLL", Action::Roll, true, 18.0), RollButton),
                        ],
                    ),
                    (Field::Status, label("", 4.6, TEXT)),
                    (Field::Event, label("", 3.4, MUTED)),
                    (
                        row(),
                        children![button("Leave game", Action::Menu, false, 8.0)]
                    ),
                ],
            ),
        ],
    ));

    commands.spawn((
        overlay(Overlay::Screen(Screen::Menu), BG),
        children![
            label("LUDO", 14.0, ACCENT),
            heading("Play on this screen"),
            (
                row(),
                children![
                    button("2 players", Action::Local(2), true, 11.0),
                    button("3 players", Action::Local(3), true, 11.0),
                    button("4 players", Action::Local(4), true, 11.0),
                ],
            ),
            heading("Play online"),
            (
                row(),
                children![
                    button("Host 2", Action::Create(2), false, 11.0),
                    button("Host 3", Action::Create(3), false, 11.0),
                    button("Host 4", Action::Create(4), false, 11.0),
                ],
            ),
            (
                row(),
                children![button("Join a room", Action::OpenJoin, false, 11.0)]
            ),
            (Field::Error, label("", 3.4, ERROR)),
        ],
    ));

    commands.spawn((
        overlay(Overlay::Screen(Screen::Join), BG),
        children![
            heading("Enter room code"),
            (Field::JoinCode, label("", 13.0, ACCENT)),
            (Field::Error, label("", 3.4, ERROR)),
            (row(), children![key(1), key(2), key(3)]),
            (row(), children![key(4), key(5), key(6)]),
            (row(), children![key(7), key(8), key(9)]),
            (
                row(),
                children![
                    button("<", Action::Backspace, false, 10.0),
                    key(0),
                    button("OK", Action::Join, true, 10.0),
                ],
            ),
            (row(), children![button("Back", Action::Menu, false, 8.0)]),
        ],
    ));

    commands.spawn((
        overlay(Overlay::Screen(Screen::Lobby), BG),
        children![
            heading("Room code: share it"),
            (Field::Room, label("", 16.0, ACCENT)),
            slot(true, 0),
            slot(true, 1),
            slot(true, 2),
            slot(true, 3),
            (Field::Wait, label("", 3.6, MUTED)),
            (row(), children![button("Leave", Action::Menu, false, 9.0)]),
        ],
    ));

    commands.spawn((
        overlay(Overlay::GameOver, Color::srgba(0.05, 0.05, 0.07, 0.9)),
        children![
            (Field::Winner, label("", 9.0, ACCENT)),
            (
                row(),
                children![button("Back to menu", Action::Menu, true, 11.0)]
            ),
        ],
    ));
}

// ---- systems ----------------------------------------------------------------

/// Portrait: board on top, panel below. Landscape: board left, panel right.
fn responsive_layout(
    window: Query<&Window, With<PrimaryWindow>>,
    mut parts: Query<(&Part, &mut Node)>,
    mut landscape: Local<Option<bool>>,
) {
    let Ok(window) = window.single() else { return };
    let wide = window.width() > window.height();
    if *landscape == Some(wide) {
        return;
    }
    *landscape = Some(wide);
    for (part, mut node) in &mut parts {
        match part {
            Part::Root => {
                node.flex_direction = if wide {
                    FlexDirection::Row
                } else {
                    FlexDirection::Column
                }
            }
            Part::Board => {
                node.width = if wide { auto() } else { percent(100) };
                node.height = if wide { percent(100) } else { auto() };
            }
            Part::Panel => {
                node.width = if wide { auto() } else { percent(100) };
                node.height = if wide { percent(100) } else { auto() };
                node.flex_grow = 1.0;
            }
        }
    }
}

fn show_overlays(session: Res<Session>, mut overlays: Query<(&Overlay, &mut Node)>) {
    if !session.is_changed() {
        return;
    }
    for (overlay, mut node) in &mut overlays {
        let visible = match overlay {
            Overlay::Screen(screen) => *screen == session.screen,
            Overlay::GameOver => session.screen == Screen::Game && session.winner().is_some(),
        };
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
    }
}

fn sync_fields(session: Res<Session>, mut fields: Query<(&Field, &mut Text)>) {
    if !session.is_changed() {
        return;
    }
    for (field, mut text) in &mut fields {
        **text = match field {
            Field::JoinCode => (0..CODE_LEN)
                .map(|i| session.join_code.chars().nth(i).unwrap_or('_').to_string())
                .collect::<Vec<_>>()
                .join(" "),
            Field::Error => session.error.clone().unwrap_or_default(),
            Field::Room => session.room.clone().unwrap_or_default(),
            Field::Wait => {
                let missing = session.capacity.saturating_sub(session.players.len());
                format!("Waiting for {missing} more...")
            }
            Field::Status => session.status(),
            Field::Event => session.last_event.clone().unwrap_or_default(),
            Field::Die => session.die().map_or("-".into(), |d| d.to_string()),
            Field::Winner => session
                .winner()
                .map_or(String::new(), |s| format!("{} wins!", session.name(s))),
        };
    }
}

fn sync_slots(
    session: Res<Session>,
    mut slots: Query<(&Slot, &mut Node, &mut BorderColor, &Children)>,
    mut dots: Query<&mut BackgroundColor, With<Dot>>,
    mut labels: Query<&mut Text, With<SlotLabel>>,
) {
    if !session.is_changed() {
        return;
    }
    let turn = session
        .game
        .as_ref()
        .filter(|g| g.winner.is_none())
        .map(|g| g.turn);
    for (slot, mut node, mut border, children) in &mut slots {
        let Some(player) = session.players.get(slot.idx) else {
            node.display = Display::None;
            continue;
        };
        node.display = Display::Flex;
        let active = !slot.lobby && turn == Some(slot.idx);
        *border = BorderColor::all(if active { ACCENT } else { Color::NONE });

        let mut name = player.player.name().to_string();
        if session.online && session.you == Some(slot.idx) {
            name.push_str(" (you)");
        }
        if !player.connected {
            name.push_str(" (left)");
        }
        for child in children {
            if let Ok(mut dot) = dots.get_mut(*child) {
                dot.0 = player_color(player.player);
            }
            if let Ok(mut text) = labels.get_mut(*child) {
                **text = name.clone();
            }
        }
    }
}

fn sync_roll_button(
    mut commands: Commands,
    session: Res<Session>,
    roll: Query<(Entity, Has<InteractionDisabled>), With<RollButton>>,
) {
    if !session.is_changed() {
        return;
    }
    for (entity, disabled) in &roll {
        match (session.can_roll(), disabled) {
            (true, true) => {
                commands.entity(entity).remove::<InteractionDisabled>();
            }
            (false, false) => {
                commands.entity(entity).insert(InteractionDisabled);
            }
            _ => {}
        }
    }
}

fn button_look(
    mut buttons: Query<
        (
            Has<Pressed>,
            Has<InteractionDisabled>,
            &Tint,
            &mut BackgroundColor,
        ),
        With<Button>,
    >,
) {
    for (pressed, disabled, tint, mut background) in &mut buttons {
        background.0 = if disabled {
            tint.0.mix(&BG, 0.6)
        } else if pressed {
            tint.0.mix(&Color::WHITE, 0.3)
        } else {
            tint.0
        };
    }
}

/// Frame the camera so the board fills its layout area, centred.
fn fit_board(
    area: Query<(&ComputedNode, &UiGlobalTransform), With<BoardArea>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut camera: Query<(&mut Projection, &mut Transform), With<Camera2d>>,
) {
    let (Ok((node, transform)), Ok(window), Ok((mut projection, mut camera_tf))) =
        (area.single(), window.single(), camera.single_mut())
    else {
        return;
    };
    let size = node.size * node.inverse_scale_factor;
    if size.min_element() < 1.0 {
        return;
    }
    let center = transform.affine().translation * node.inverse_scale_factor;
    let (w, h) = (window.width(), window.height());
    // World units per logical pixel, and where the area centre sits relative to the window centre.
    let k = EXTENT / size.min_element();
    let offset = (center - Vec2::new(w, h) / 2.0) * k;

    let (width, height) = (w * k, h * k);
    match &mut *projection {
        Projection::Orthographic(o) if matches!(o.scaling_mode, ScalingMode::Fixed { width: fw, height: fh } if fw == width && fh == height) =>
            {}
        Projection::Orthographic(o) => o.scaling_mode = ScalingMode::Fixed { width, height },
        other => {
            *other = Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::Fixed { width, height },
                ..OrthographicProjection::default_2d()
            })
        }
    }
    let target = Vec3::new(-offset.x, offset.y, camera_tf.translation.z);
    if camera_tf.translation != target {
        camera_tf.translation = target;
    }
}
