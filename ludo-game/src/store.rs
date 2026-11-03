//! The player's profile, kept in the browser's local storage.

use ludo_core::Player;
use serde::{Deserialize, Serialize};

const KEY: &str = "ludo.profile";

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub color: Player,
    /// Stable id that lets the server give a seat back after a reconnect.
    pub client_id: String,
}

const SETTINGS_KEY: &str = "ludo.settings";

/// Display preferences, kept in local storage.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Particles, confetti and the turn banner.
    pub effects: bool,
    /// Skip jumps, tumbling and bobbing, and play moves faster.
    pub reduced_motion: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            effects: true,
            reduced_motion: false,
        }
    }
}

/// Saved settings; a first visit follows the system's reduced-motion preference.
pub fn load_settings() -> Settings {
    let saved = storage()
        .and_then(|s| s.get_item(SETTINGS_KEY).ok()?)
        .and_then(|json| serde_json::from_str(&json).ok());
    saved.unwrap_or_else(|| Settings {
        reduced_motion: prefers_reduced_motion(),
        ..Settings::default()
    })
}

pub fn save_settings(settings: &Settings) {
    if let (Some(storage), Ok(json)) = (storage(), serde_json::to_string(settings)) {
        let _ = storage.set_item(SETTINGS_KEY, &json);
    }
}

fn prefers_reduced_motion() -> bool {
    web_sys::window()
        .and_then(|w| w.match_media("(prefers-reduced-motion: reduce)").ok()?)
        .is_some_and(|m| m.matches())
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub fn load() -> Option<Profile> {
    serde_json::from_str(&storage()?.get_item(KEY).ok()??).ok()
}

pub fn save(profile: &Profile) {
    if let (Some(storage), Ok(json)) = (storage(), serde_json::to_string(profile)) {
        let _ = storage.set_item(KEY, &json);
    }
}

fn random(n: usize) -> usize {
    (js_sys::Math::random() * n as f64) as usize % n
}

/// A fresh profile with a friendly random name and colour.
pub fn random_profile() -> Profile {
    const ADJECTIVES: [&str; 8] = [
        "Swift", "Lucky", "Brave", "Sly", "Jolly", "Calm", "Bold", "Merry",
    ];
    const ANIMALS: [&str; 8] = [
        "Fox", "Otter", "Panda", "Hawk", "Tiger", "Koala", "Lynx", "Crane",
    ];
    Profile {
        name: format!("{} {}", ADJECTIVES[random(8)], ANIMALS[random(8)]),
        color: Player::ALL[random(4)],
        client_id: format!(
            "{:08x}{:08x}{:08x}",
            random(1 << 31),
            random(1 << 31),
            random(1 << 31)
        ),
    }
}
