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
