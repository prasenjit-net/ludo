//! Browser WebSocket connection to the game server.

use ludo_core::protocol::{ClientMsg, ServerMsg};
use std::{cell::RefCell, collections::VecDeque};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{CloseEvent, MessageEvent, WebSocket};

pub enum NetEvent {
    Msg(ServerMsg),
    Closed,
}

struct Conn {
    ws: WebSocket,
    _closures: (
        Closure<dyn FnMut()>,
        Closure<dyn FnMut(MessageEvent)>,
        Closure<dyn FnMut(CloseEvent)>,
    ),
}

thread_local! {
    static CONN: RefCell<Option<Conn>> = const { RefCell::new(None) };
    static INBOX: RefCell<VecDeque<NetEvent>> = const { RefCell::new(VecDeque::new()) };
}

fn push(ev: NetEvent) {
    INBOX.with(|q| q.borrow_mut().push_back(ev));
}

/// Open a socket and send `first` as soon as it is connected.
pub fn connect(first: &ClientMsg) -> Result<(), String> {
    close();
    let loc = web_sys::window().ok_or("no window")?.location();
    let secure = loc.protocol().map_err(|_| "no protocol")? == "https:";
    let host = loc.host().map_err(|_| "no host")?;
    let url = format!("{}://{host}/ws", if secure { "wss" } else { "ws" });
    let ws = WebSocket::new(&url).map_err(|_| format!("cannot open {url}"))?;

    let first = serde_json::to_string(first).map_err(|e| e.to_string())?;
    let ws_open = ws.clone();
    let on_open = Closure::<dyn FnMut()>::new(move || {
        let _ = ws_open.send_with_str(&first);
    });
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        if let Some(text) = e.data().as_string()
            && let Ok(msg) = serde_json::from_str(&text)
        {
            push(NetEvent::Msg(msg));
        }
    });
    let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |_| push(NetEvent::Closed));

    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    CONN.with(|c| *c.borrow_mut() = Some(Conn { ws, _closures: (on_open, on_message, on_close) }));
    Ok(())
}

pub fn send(msg: &ClientMsg) {
    CONN.with(|c| {
        if let (Some(conn), Ok(text)) = (c.borrow().as_ref(), serde_json::to_string(msg)) {
            let _ = conn.ws.send_with_str(&text);
        }
    });
}

/// Close the socket without reporting it as a connection loss.
pub fn close() {
    CONN.with(|c| {
        if let Some(conn) = c.borrow_mut().take() {
            conn.ws.set_onclose(None);
            conn.ws.set_onmessage(None);
            let _ = conn.ws.close();
        }
    });
    INBOX.with(|q| q.borrow_mut().clear());
}

pub fn poll() -> Vec<NetEvent> {
    INBOX.with(|q| q.borrow_mut().drain(..).collect())
}
