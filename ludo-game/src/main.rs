// Bevy systems take many parameters and queries have long types by design.
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

mod board;
mod dice;
mod net;
mod paint;
mod play;
mod session;
mod store;
mod ui;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Ludo".into(),

                fit_canvas_to_parent: true,
                prevent_default_event_handling: true,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.12, 0.12, 0.14)))
        .add_plugins((
            board::BoardPlugin,
            dice::DicePlugin,
            play::PlayPlugin,
            ui::UiPlugin,
        ))
        .run();
}
