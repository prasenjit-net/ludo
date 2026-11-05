//! A way to type on phones. The text boxes are drawn by Bevy on a canvas, and a
//! browser only opens its on-screen keyboard for a real, focused `<input>`. So on
//! touch screens a tap on a text box focuses a transparent `<input>` placed over
//! it, and what is typed there is copied into the Bevy box.
//!
//! The focus has to happen inside the tap's own event handler (browsers refuse
//! otherwise), which is why the tap is handled in a DOM listener and not in a
//! Bevy system. Desktops keep using plain key events and never get this.

use crate::{
    session::{NAME_MAX, ROOM_NAME_MAX, Screen, Session},
    ui::Input,
};
use bevy::window::PrimaryWindow;
use bevy::{
    prelude::*,
    text::{EditableText, TextEdit},
    ui::{ComputedNode, UiGlobalTransform},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{HtmlInputElement, KeyboardEvent, MouseEvent, TouchEvent};

pub struct KeyboardPlugin;

impl Plugin for KeyboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (install, publish_boxes, apply_typed).chain());
    }
}

/// A text box as the DOM sees it: where it is on screen (CSS pixels) and what it holds.
#[derive(Clone)]
struct BoxInfo {
    kind: Input,
    /// Left, top, width, height.
    rect: [f32; 4],
    value: String,
    max: u32,
    capitalize: &'static str,
}

#[derive(Default)]
struct Bridge {
    boxes: Vec<BoxInfo>,
    /// The box the DOM input is editing.
    active: Option<Input>,
    /// Text typed into the DOM input, waiting to be copied into Bevy.
    typed: Vec<(Input, String)>,
    input: Option<HtmlInputElement>,
    canvas: Option<web_sys::Element>,
}

thread_local! {
    static BRIDGE: RefCell<Bridge> = RefCell::default();
}

fn coarse_pointer() -> bool {
    web_sys::window()
        .and_then(|w| w.match_media("(pointer: coarse)").ok()?)
        .is_some_and(|m| m.matches())
}

#[derive(Default, PartialEq)]
enum Setup {
    #[default]
    Pending,
    Done,
    /// Not a touch screen: nothing to do.
    Skipped,
}

/// Create the `<input>` and the tap listener once the canvas exists.
fn install(mut setup: Local<Setup>) {
    if *setup != Setup::Pending {
        return;
    }
    if !coarse_pointer() {
        *setup = Setup::Skipped;
        return;
    }
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(Some(canvas)) = document.query_selector("canvas") else {
        return; // not created yet; try again next frame
    };
    let Some(body) = document.body() else { return };
    let Some(input) = document
        .create_element("input")
        .ok()
        .and_then(|e| e.dyn_into::<HtmlInputElement>().ok())
    else {
        return;
    };

    for (name, value) in [
        ("type", "text"),
        ("autocomplete", "off"),
        ("autocorrect", "off"),
        ("spellcheck", "false"),
        ("enterkeyhint", "done"),
        ("aria-label", "Text entry"),
    ] {
        let _ = input.set_attribute(name, value);
    }
    let style = input.style();
    for (name, value) in [
        ("position", "fixed"),
        ("left", "0"),
        ("top", "0"),
        ("width", "1px"),
        ("height", "1px"),
        // Invisible but still focusable; 16px stops iOS zooming in on focus.
        ("opacity", "0"),
        ("font-size", "16px"),
        ("border", "0"),
        ("padding", "0"),
        ("pointer-events", "none"),
    ] {
        let _ = style.set_property(name, value);
    }
    let _ = body.append_child(&input);

    // Typing: copy the text for Bevy to pick up.
    let typed = input.clone();
    let on_input = Closure::<dyn FnMut()>::new(move || {
        let value = typed.value();
        BRIDGE.with(|b| {
            let mut b = b.borrow_mut();
            if let Some(kind) = b.active {
                b.typed.push((kind, value));
            }
        });
    });
    let _ = input.add_event_listener_with_callback("input", on_input.as_ref().unchecked_ref());
    on_input.forget();

    // Enter closes the keyboard.
    let done = input.clone();
    let on_key = Closure::<dyn FnMut(KeyboardEvent)>::new(move |e: KeyboardEvent| {
        if e.key() == "Enter" {
            let _ = done.blur();
        }
    });
    let _ = input.add_event_listener_with_callback("keydown", on_key.as_ref().unchecked_ref());
    on_key.forget();

    let on_blur = Closure::<dyn FnMut()>::new(|| {
        BRIDGE.with(|b| b.borrow_mut().active = None);
    });
    let _ = input.add_event_listener_with_callback("blur", on_blur.as_ref().unchecked_ref());
    on_blur.forget();

    // A tap on a text box opens the keyboard; a tap elsewhere closes it. Which
    // event reaches us depends on the browser (Bevy cancels some touch defaults,
    // which can swallow `click`), so listen for all three and handle a tap once.
    let last_tap = Rc::new(Cell::new(f64::MIN));
    for name in ["pointerup", "touchend", "click"] {
        let (target, last_tap) = (input.clone(), last_tap.clone());
        let on_tap = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            let Some((x, y)) = tap_position(&e) else {
                return;
            };
            if e.time_stamp() - last_tap.get() < 400.0 {
                return;
            }
            last_tap.set(e.time_stamp());
            tap(&target, x, y);
        });
        let _ = canvas.add_event_listener_with_callback(name, on_tap.as_ref().unchecked_ref());
        on_tap.forget();
    }

    BRIDGE.with(|b| {
        let mut b = b.borrow_mut();
        b.input = Some(input);
        b.canvas = Some(canvas);
    });
    *setup = Setup::Done;
}

/// Where a pointer, touch or click event happened, in CSS pixels.
fn tap_position(e: &web_sys::Event) -> Option<(f32, f32)> {
    if let Some(touch) = e.dyn_ref::<TouchEvent>() {
        let t = touch.changed_touches().get(0)?;
        return Some((t.client_x() as f32, t.client_y() as f32));
    }
    let m = e.dyn_ref::<MouseEvent>()?;
    Some((m.client_x() as f32, m.client_y() as f32))
}

/// Open the keyboard on the text box at this point, or close it if there is none.
fn tap(input: &HtmlInputElement, x: f32, y: f32) {
    let hit = BRIDGE.with(|b| {
        b.borrow()
            .boxes
            .iter()
            .find(|bx| {
                let [l, t, w, h] = bx.rect;
                x >= l - 6.0 && x <= l + w + 6.0 && y >= t - 6.0 && y <= t + h + 6.0
            })
            .cloned()
    });
    // The borrow is released: focusing and blurring fire events that use it.
    let Some(bx) = hit else {
        let _ = input.blur();
        return;
    };
    BRIDGE.with(|b| b.borrow_mut().active = Some(bx.kind));
    let [l, t, w, h] = bx.rect;
    let style = input.style();
    for (name, value) in [
        ("left", format!("{l}px")),
        ("top", format!("{t}px")),
        ("width", format!("{w}px")),
        ("height", format!("{h}px")),
    ] {
        let _ = style.set_property(name, &value);
    }
    let _ = input.set_attribute("maxlength", &bx.max.to_string());
    let _ = input.set_attribute("autocapitalize", bx.capitalize);
    input.set_value(&bx.value);
    let _ = input.focus();
    let end = bx.value.chars().count() as u32;
    let _ = input.set_selection_range(end, end);
}

/// Tell the DOM side where the visible text boxes are and what they contain.
fn publish_boxes(
    session: Res<Session>,
    window: Query<&Window, With<PrimaryWindow>>,
    inputs: Query<(&Input, &ComputedNode, &UiGlobalTransform, &EditableText)>,
) {
    if BRIDGE.with(|b| b.borrow().input.is_none()) {
        return; // not installed: a desktop, or the canvas is not up yet
    }
    // Layout values are in the window's own pixels; the DOM wants CSS pixels.
    let css_per_px = BRIDGE
        .with(|b| b.borrow().canvas.as_ref().map(|c| c.client_width() as f32))
        .zip(window.single().ok().map(|w| w.physical_width() as f32))
        .filter(|(_, px)| *px > 0.0)
        .map_or(1.0, |(css, px)| css / px);
    let mut boxes = Vec::new();
    for (kind, node, transform, text) in &inputs {
        let visible = matches!(
            (kind, session.screen),
            (Input::Name, Screen::Profile) | (Input::RoomName, Screen::Create)
        );
        let size = node.size * css_per_px;
        if !visible || size.min_element() < 1.0 {
            continue;
        }
        let center = transform.affine().translation * css_per_px;
        let (max, capitalize) = match kind {
            Input::Name => (NAME_MAX, "words"),
            Input::RoomName => (ROOM_NAME_MAX, "sentences"),
        };
        boxes.push(BoxInfo {
            kind: *kind,
            rect: [
                center.x - size.x / 2.0,
                center.y - size.y / 2.0,
                size.x,
                size.y,
            ],
            value: text.value().to_string(),
            max: max as u32,
            capitalize,
        });
    }
    // The box went away (a screen change): put the keyboard away too.
    let close = BRIDGE.with(|b| {
        let mut b = b.borrow_mut();
        b.boxes = boxes;
        match (&b.input, b.active, b.boxes.is_empty()) {
            (Some(input), Some(_), true) => Some(input.clone()),
            _ => None,
        }
    });
    if let Some(input) = close {
        let _ = input.blur();
    }
}

/// Copy what was typed in the DOM input into the Bevy text box.
fn apply_typed(mut inputs: Query<(&Input, &mut EditableText)>) {
    let typed = BRIDGE.with(|b| std::mem::take(&mut b.borrow_mut().typed));
    for (kind, value) in typed {
        for (k, mut text) in &mut inputs {
            if *k == kind && text.value().to_string() != value {
                text.editor_mut().set_text(&value);
                text.queue_edit(TextEdit::TextEnd(false));
            }
        }
    }
}
