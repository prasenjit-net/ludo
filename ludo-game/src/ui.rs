//! All user interface, built with `bevy_ui`. Sizes use `vmin` so the layout
//! scales with the screen; portrait (phone) is the primary layout, and
//! landscape moves the control panel beside the board.

use crate::{
    board::{EXTENT, player_color},
    session::{Action, Screen, Session},
};
use bevy::{
    camera::ScalingMode,
    color::Mix,
    input_focus::tab_navigation::TabIndex,
    prelude::*,
    text::{EditableText, TextCursorStyle, TextEdit},
    ui::{InteractionDisabled, Pressed, UiGlobalTransform},
    ui_widgets::{Activate, Button, TextInput, observe},
    window::PrimaryWindow,
};
use ludo_core::{Phase, Player};

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
                (fill_inputs, read_inputs).chain(),
                responsive_layout,
                show_overlays,
                sync_fields,
                sync_slots,
                sync_rows,
                sync_swatches,
                sync_controls,
                countdown,
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
    Error,
    ConnLine,
    Empty,
    Page,
    ProfileName,
    RowTitle(usize),
    RowSub(usize),
    RoomTitle,
    RoomHint,
    Status,
    Event,
    Timer,
    Die,
    Winner,
}

/// A text box and what it edits.
#[derive(Component, Clone, Copy, PartialEq)]
enum Input {
    Name,
    RoomName,
}

/// A player chip; `lobby` chips live in the waiting room, the others in the game panel.
#[derive(Component)]
struct Slot {
    lobby: bool,
    idx: usize,
}

#[derive(Component)]
struct Dot;

#[derive(Component)]
struct ProfileDot;

#[derive(Component)]
struct SlotLabel;

/// A colour choice button; `in_room` swatches change the colour in the waiting room.
#[derive(Component)]
struct Swatch {
    player: Player,
    in_room: bool,
}

/// One row of the room browser, and its Join button.
#[derive(Component)]
struct RoomRow(usize);

#[derive(Component)]
struct RowJoin(usize);

/// "2 / 3 / 4 players" choice on the create screen.
#[derive(Component)]
struct Choice(u8);

#[derive(Component)]
struct RollButton;

#[derive(Component)]
struct StartButton;

/// Shown only while offline.
#[derive(Component)]
struct RetryBar;

/// Shown only when there is a profile to go back to.
#[derive(Component)]
struct BackBar;

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

/// A button; `width` is in vmin, or `None` to share the row equally.
fn button_w(
    text: impl Into<String>,
    action: Action,
    primary: bool,
    height: f32,
    width: Option<f32>,
) -> impl Bundle {
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
            flex_grow: if width.is_some() { 0.0 } else { 1.0 },
            flex_basis: if width.is_some() { auto() } else { px(0) },
            width: width.map_or(auto(), vmin),
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

fn button(text: impl Into<String>, action: Action, primary: bool, height: f32) -> impl Bundle {
    button_w(text, action, primary, height, None)
}

fn row() -> Node {
    Node {
        width: percent(100),
        max_width: vmin(90),
        column_gap: vmin(2.0),
        align_items: AlignItems::Center,
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
            row_gap: vmin(2.0),
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

fn text_input(kind: Input) -> impl Bundle {
    (
        kind,
        TextInput,
        // Needed for a tap to focus the box.
        TabIndex(0),
        EditableText {
            allow_newlines: false,
            ..default()
        },
        TextLayout::no_wrap(),
        TextFont {
            font_size: FontSize::VMin(5.0),
            ..default()
        },
        TextColor(TEXT),
        TextCursorStyle::default(),
        Node {
            width: percent(100),
            max_width: vmin(90),
            height: vmin(11),
            padding: UiRect::axes(vmin(3.0), vmin(2.0)),
            border: UiRect::all(vmin(0.6)),
            border_radius: BorderRadius::all(vmin(2.0)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(CHIP),
        BorderColor::all(MUTED),
    )
}

fn dot(size: f32) -> Node {
    Node {
        width: vmin(size),
        height: vmin(size),
        border_radius: BorderRadius::MAX,
        ..default()
    }
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
            (Dot, dot(4.0), BackgroundColor(Color::WHITE)),
            (SlotLabel, label("", 3.4, TEXT))
        ],
    )
}

fn swatch(player: Player, in_room: bool) -> impl Bundle {
    (
        Button,
        Swatch { player, in_room },
        Node {
            width: vmin(13.0),
            height: vmin(13.0),
            border: UiRect::all(vmin(1.0)),
            border_radius: BorderRadius::all(vmin(2.5)),
            ..default()
        },
        BackgroundColor(player_color(player)),
        BorderColor::all(Color::NONE),
        observe(move |_: On<Activate>, mut session: ResMut<Session>| {
            session.apply(Action::PickColor(player))
        }),
    )
}

fn swatches(in_room: bool) -> impl Bundle {
    (
        Node {
            column_gap: vmin(3.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        children![
            swatch(Player::Red, in_room),
            swatch(Player::Green, in_room),
            swatch(Player::Yellow, in_room),
            swatch(Player::Blue, in_room),
        ],
    )
}

fn choice(n: u8) -> impl Bundle {
    (
        Choice(n),
        button(format!("{n} players"), Action::SetPlayers(n), false, 10.0),
    )
}

fn room_row(i: usize) -> impl Bundle {
    (
        RoomRow(i),
        Node {
            width: percent(100),
            max_width: vmin(90),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            column_gap: vmin(2.0),
            padding: UiRect::axes(vmin(3.0), vmin(1.5)),
            border_radius: BorderRadius::all(vmin(2.0)),
            ..default()
        },
        BackgroundColor(PANEL),
        children![
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    flex_shrink: 1.0,
                    ..default()
                },
                children![
                    (Field::RowTitle(i), label("", 4.4, TEXT)),
                    (Field::RowSub(i), label("", 3.0, MUTED))
                ],
            ),
            (
                RowJoin(i),
                button_w("Join", Action::JoinRow(i), true, 8.0, Some(20.0))
            ),
        ],
    )
}

fn profile_chip() -> impl Bundle {
    (
        Button,
        Tint(BUTTON),
        BackgroundColor(BUTTON),
        Node {
            align_items: AlignItems::Center,
            column_gap: vmin(1.5),
            padding: UiRect::axes(vmin(2.5), vmin(1.5)),
            border_radius: BorderRadius::all(vmin(2.0)),
            ..default()
        },
        observe(|_: On<Activate>, mut session: ResMut<Session>| session.apply(Action::OpenProfile)),
        children![
            (ProfileDot, dot(4.0), BackgroundColor(Color::WHITE)),
            (Field::ProfileName, label("", 3.6, TEXT))
        ],
    )
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
                    // Without this the aspect ratio forbids shrinking below a square.
                    min_width: px(0),
                    min_height: px(0),
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
                    (Field::Timer, label("", 3.4, ACCENT)),
                    (Field::Event, label("", 3.4, MUTED)),
                    (
                        row(),
                        children![button("Leave game", Action::Leave, false, 8.0)]
                    ),
                ],
            ),
        ],
    ));

    // Profile: name and colour.
    commands.spawn((
        overlay(Overlay::Screen(Screen::Profile), BG),
        children![
            label("YOUR PROFILE", 7.0, ACCENT),
            heading("Name"),
            text_input(Input::Name),
            heading("Colour"),
            swatches(false),
            (
                row(),
                children![button("Continue", Action::SaveProfile, true, 11.0)]
            ),
            (
                BackBar,
                row(),
                children![button("Back", Action::Back, false, 8.0)]
            ),
            (Field::Error, label("", 3.4, ERROR)),
        ],
    ));

    // Room browser: the first screen.
    commands.spawn((
        overlay(Overlay::Screen(Screen::Rooms), BG),
        children![
            (
                Node {
                    width: percent(100),
                    max_width: vmin(90),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    ..default()
                },
                children![label("ROOMS", 7.0, ACCENT), profile_chip()],
            ),
            (Field::Empty, label("", 3.6, MUTED)),
            room_row(0),
            room_row(1),
            room_row(2),
            (
                row(),
                children![
                    button_w("<", Action::PagePrev, false, 7.0, Some(14.0)),
                    (Field::Page, label("", 3.6, MUTED)),
                    button_w(">", Action::PageNext, false, 7.0, Some(14.0)),
                ],
            ),
            (Field::ConnLine, label("", 3.4, MUTED)),
            (
                RetryBar,
                row(),
                children![button("Retry now", Action::Retry, false, 7.0)]
            ),
            (
                row(),
                children![button("Create a room", Action::OpenCreate, true, 10.0)]
            ),
            heading("Same screen"),
            (
                row(),
                children![
                    button("2 players", Action::Local(2), false, 8.0),
                    button("3 players", Action::Local(3), false, 8.0),
                    button("4 players", Action::Local(4), false, 8.0),
                ],
            ),
            (Field::Error, label("", 3.4, ERROR)),
        ],
    ));

    commands.spawn((
        overlay(Overlay::Screen(Screen::Create), BG),
        children![
            label("NEW ROOM", 7.0, ACCENT),
            heading("Room name"),
            text_input(Input::RoomName),
            heading("Players allowed"),
            (row(), children![choice(2), choice(3), choice(4)]),
            (
                row(),
                children![button("Create", Action::CreateRoom, true, 11.0)]
            ),
            (row(), children![button("Back", Action::Back, false, 8.0)]),
            (Field::Error, label("", 3.4, ERROR)),
        ],
    ));

    // Waiting room.
    commands.spawn((
        overlay(Overlay::Screen(Screen::Room), BG),
        children![
            (Field::RoomTitle, label("", 6.0, ACCENT)),
            slot(true, 0),
            slot(true, 1),
            slot(true, 2),
            slot(true, 3),
            heading("Your colour"),
            swatches(true),
            (Field::RoomHint, label("", 3.6, MUTED)),
            (
                StartButton,
                row(),
                children![button("Start game", Action::Start, true, 11.0)]
            ),
            (row(), children![button("Leave", Action::Leave, false, 8.0)]),
            (Field::Error, label("", 3.4, ERROR)),
        ],
    ));

    commands.spawn((
        overlay(Overlay::GameOver, Color::srgba(0.05, 0.05, 0.07, 0.9)),
        children![
            (Field::Winner, label("", 9.0, ACCENT)),
            (
                row(),
                children![button("Back to rooms", Action::Leave, true, 11.0)]
            ),
        ],
    ));
}

// ---- systems ----------------------------------------------------------------

/// Put the draft text into the box when its screen opens.
fn fill_inputs(
    session: Res<Session>,
    mut inputs: Query<(&Input, &mut EditableText)>,
    mut shown: Local<Option<Screen>>,
) {
    if *shown == Some(session.screen) {
        return;
    }
    *shown = Some(session.screen);
    for (kind, mut text) in &mut inputs {
        let value = match (kind, session.screen) {
            (Input::Name, Screen::Profile) => &session.name_draft,
            (Input::RoomName, Screen::Create) => &session.room_draft,
            _ => continue,
        };
        text.editor_mut().set_text(value);
        text.queue_edit(TextEdit::TextEnd(false));
    }
}

/// Copy what the user typed back into the session (without waking the other UI systems).
fn read_inputs(
    mut session: ResMut<Session>,
    inputs: Query<(&Input, &EditableText), Changed<EditableText>>,
) {
    for (kind, text) in &inputs {
        let value = text.value().to_string();
        let session = session.bypass_change_detection();
        match kind {
            Input::Name => session.name_draft = value,
            Input::RoomName => session.room_draft = value,
        }
    }
}

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

fn sync_fields(
    session: Res<Session>,
    mut fields: Query<(&Field, &mut Text)>,
    mut dots: Query<&mut BackgroundColor, With<ProfileDot>>,
) {
    if !session.is_changed() {
        return;
    }
    for mut dot in &mut dots {
        dot.0 = player_color(session.profile.color);
    }
    for (field, mut text) in &mut fields {
        let value = match field {
            Field::Error => session.error.clone().unwrap_or_default(),
            Field::ConnLine => match (session.connected, session.reconnect_in) {
                (true, _) => String::new(),
                (false, Some(_)) => "Offline - trying to reconnect...".into(),
                (false, None) => "Connecting...".into(),
            },
            Field::Empty => match (session.connected, session.rooms.is_empty()) {
                (true, true) => "No open rooms yet. Create one!".into(),
                _ => String::new(),
            },
            Field::Page => format!("{} / {}", session.page + 1, session.page_count()),
            Field::ProfileName => session.profile.name.clone(),
            Field::RowTitle(i) => session.row(*i).map(|r| r.name.clone()).unwrap_or_default(),
            Field::RowSub(i) => session
                .row(*i)
                .map(|r| {
                    let state = if r.playing {
                        "In game"
                    } else if r.joined >= r.capacity {
                        "Full"
                    } else {
                        "Waiting"
                    };
                    format!("{} - {}/{} players - {state}", r.host, r.joined, r.capacity)
                })
                .unwrap_or_default(),
            Field::RoomTitle => session
                .room
                .as_ref()
                .map(|r| r.name.clone())
                .unwrap_or_default(),
            Field::RoomHint => match (&session.room, session.is_host()) {
                (Some(_), false) => "Waiting for the host to start".into(),
                (Some(_), true) if session.players.len() < 2 => {
                    "Waiting for another player to join".into()
                }
                (Some(_), true) => "Start now, or wait for more players".into(),
                _ => String::new(),
            },
            Field::Status => session.status(),
            Field::Event => session.last_event.clone().unwrap_or_default(),
            Field::Die => session.die().map_or("-".into(), |d| d.to_string()),
            Field::Winner => session
                .winner()
                .map_or(String::new(), |s| format!("{} wins!", session.name(s))),
            Field::Timer => continue,
        };
        **text = value;
    }
}

fn sync_slots(
    session: Res<Session>,
    mut slots: Query<
        (
            &Slot,
            &mut Node,
            &mut BorderColor,
            &mut BackgroundColor,
            &Children,
        ),
        Without<Dot>,
    >,
    mut dots: Query<&mut BackgroundColor, With<Dot>>,
    mut labels: Query<(&mut Text, &mut TextColor), With<SlotLabel>>,
) {
    if !session.is_changed() {
        return;
    }
    let turn = session
        .game
        .as_ref()
        .filter(|g| g.winner.is_none())
        .map(|g| g.turn);
    let capacity = session.room.as_ref().map_or(0, |r| r.capacity);
    for (slot, mut node, mut border, mut background, children) in &mut slots {
        let player = session.players.get(slot.idx);
        // Waiting rooms also show the open seats.
        if player.is_none() && !(slot.lobby && slot.idx < capacity) {
            node.display = Display::None;
            continue;
        }
        node.display = Display::Flex;
        let left = session
            .game
            .as_ref()
            .is_some_and(|g| g.left.get(slot.idx).copied().unwrap_or(false));
        let active = !slot.lobby && turn == Some(slot.idx);
        *border = BorderColor::all(if active { ACCENT } else { Color::NONE });
        background.0 = if left { CHIP.mix(&PANEL, 0.5) } else { CHIP };

        let (mut name, color, text_color) = match player {
            Some(p) => (
                p.name.clone(),
                player_color(p.player),
                if left { MUTED } else { TEXT },
            ),
            None => ("Open seat".to_string(), BUTTON, MUTED),
        };
        if let Some(p) = player {
            if slot.lobby && session.room.as_ref().is_some_and(|r| r.host == slot.idx) {
                name.push_str(" (host)");
            }
            if session.in_room && session.you == Some(slot.idx) {
                name.push_str(" (you)");
            }
            if left {
                name.push_str(" (left)");
            } else if !p.connected {
                name.push_str(" (away)");
            }
        }
        for child in children {
            if let Ok(mut dot) = dots.get_mut(*child) {
                dot.0 = color;
            }
            if let Ok((mut text, mut tc)) = labels.get_mut(*child) {
                **text = name.clone();
                tc.0 = text_color;
            }
        }
    }
}

/// Room browser rows: visible when there is a room on that row, Join enabled when it can be joined.
fn sync_rows(
    mut commands: Commands,
    session: Res<Session>,
    mut rows: Query<(&RoomRow, &mut Visibility)>,
    joins: Query<(Entity, &RowJoin, Has<InteractionDisabled>)>,
) {
    if !session.is_changed() {
        return;
    }
    // Empty rows keep their space so the pager does not jump between pages.
    for (row, mut visibility) in &mut rows {
        *visibility = if session.row(row.0).is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for (entity, join, disabled) in &joins {
        let joinable = session.connected
            && session
                .row(join.0)
                .is_some_and(|r| !r.playing && r.joined < r.capacity);
        match (joinable, disabled) {
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

/// Mark the chosen colour; in the waiting room, dim colours other players hold.
fn sync_swatches(
    session: Res<Session>,
    mut swatches: Query<(&Swatch, &mut BorderColor, &mut BackgroundColor)>,
) {
    if !session.is_changed() {
        return;
    }
    for (swatch, mut border, mut background) in &mut swatches {
        let (selected, taken) = if swatch.in_room {
            let mine = session
                .you
                .and_then(|y| session.players.get(y))
                .map(|p| p.player);
            let taken = session.players.iter().any(|p| p.player == swatch.player)
                && mine != Some(swatch.player);
            (mine == Some(swatch.player), taken)
        } else {
            (session.color_draft == swatch.player, false)
        };
        *border = BorderColor::all(if selected { TEXT } else { Color::NONE });
        let base = player_color(swatch.player);
        background.0 = if taken { base.mix(&BG, 0.7) } else { base };
    }
}

/// Buttons and bars whose state depends on the session.
#[allow(clippy::too_many_arguments)]
fn sync_controls(
    mut commands: Commands,
    session: Res<Session>,
    roll: Query<(Entity, Has<InteractionDisabled>), With<RollButton>>,
    start: Query<(Entity, &Children), With<StartButton>>,
    start_buttons: Query<Has<InteractionDisabled>, With<Button>>,
    mut nodes: ParamSet<(
        Query<&mut Node, With<RetryBar>>,
        Query<&mut Node, With<BackBar>>,
        Query<&mut Node, With<StartButton>>,
    )>,
    mut choices: Query<(&Choice, &mut Tint)>,
) {
    if !session.is_changed() {
        return;
    }
    set_disabled(&mut commands, roll.iter(), !session.can_roll());
    for mut n in &mut nodes.p0() {
        n.display = if session.connected {
            Display::None
        } else {
            Display::Flex
        };
    }
    for mut n in &mut nodes.p1() {
        n.display = if session.has_profile {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut n in &mut nodes.p2() {
        n.display = if session.is_host() {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (_, children) in &start {
        for child in children {
            if let Ok(disabled) = start_buttons.get(*child) {
                set_disabled(
                    &mut commands,
                    [(*child, disabled)],
                    session.players.len() < 2,
                );
            }
        }
    }
    for (choice, mut tint) in &mut choices {
        tint.0 = if choice.0 == session.create_players {
            ACCENT
        } else {
            BUTTON
        };
    }
}

fn set_disabled(
    commands: &mut Commands,
    buttons: impl IntoIterator<Item = (Entity, bool)>,
    want: bool,
) {
    for (entity, disabled) in buttons {
        match (want, disabled) {
            (false, true) => {
                commands.entity(entity).remove::<InteractionDisabled>();
            }
            (true, false) => {
                commands.entity(entity).insert(InteractionDisabled);
            }
            _ => {}
        }
    }
}

/// Count down to the server's automatic turn, and show it.
fn countdown(
    time: Res<Time>,
    session: Res<Session>,
    mut fields: Query<(&Field, &mut Text)>,
    mut state: Local<(u32, Option<f32>, Option<i32>)>,
) {
    if state.0 != session.turn_rev {
        state.0 = session.turn_rev;
        state.1 = session.turn_secs;
    }
    if let Some(left) = &mut state.1 {
        *left -= time.delta_secs();
    }
    let running = session.in_room
        && session
            .game
            .as_ref()
            .is_some_and(|g| g.phase() != Phase::Over);
    let secs = if running {
        state.1.map(|l| l.max(0.0).ceil() as i32)
    } else {
        None
    };
    if secs != state.2 {
        state.2 = secs;
        for (field, mut text) in &mut fields {
            if matches!(field, Field::Timer) {
                **text = secs.map_or(String::new(), |s| format!("Auto-play in {s}s"));
            }
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
