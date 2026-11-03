mod board;
mod net;
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
        .add_plugins((board::BoardPlugin, play::PlayPlugin, ui::UiPlugin))
        .run();
}
