//! Thin bridge to the HTML layer (menu, lobby, HUD), which lives in index.html.
//!
//! JS -> Rust: `window.ludoDrain()` returns queued UI commands as a JSON array.
//! Rust -> JS: `window.ludoRender(json)` receives the UI state to display.

use js_sys::{Function, Reflect};
use serde::Deserialize;
use wasm_bindgen::{JsCast, JsValue};

#[derive(Deserialize, Debug)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Cmd {
    Local { players: u8 },
    Create { name: String, players: u8 },
    Join { name: String, code: String },
    Roll,
    Menu,
}

fn global_fn(name: &str) -> Option<Function> {
    let win = web_sys::window()?;
    Reflect::get(&win, &JsValue::from_str(name)).ok()?.dyn_into().ok()
}

pub fn drain() -> Vec<Cmd> {
    let Some(f) = global_fn("ludoDrain") else { return Vec::new() };
    f.call0(&JsValue::NULL)
        .ok()
        .and_then(|v| v.as_string())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn render(json: &str) {
    if let Some(f) = global_fn("ludoRender") {
        let _ = f.call1(&JsValue::NULL, &JsValue::from_str(json));
    }
}

pub fn random_die() -> u8 {
    (js_sys::Math::random() * 6.0) as u8 % 6 + 1
}
