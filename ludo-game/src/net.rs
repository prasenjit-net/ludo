//! Browser WebSocket connection to the game server, kept alive for as long as
//! the page is: it reconnects by itself, on browser timers and events rather
//! than the game loop, which stops while the tab is hidden.

use ludo_core::{
    link::{Command, Link},
    protocol::{ClientMsg, ServerMsg},
};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{CloseEvent, MessageEvent, PageTransitionEvent, WebSocket};

/// How often timeouts and retries are checked.
const TICK_MS: i32 = 250;

pub enum NetEvent {
    Open,
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
    static LINK: RefCell<Link> = const { RefCell::new(Link::new()) };
    /// With `?chaos=1`: when to cut the connection next.
    static CHAOS: Cell<Option<f64>> = const { Cell::new(None) };
}

fn push(ev: NetEvent) {
    INBOX.with(|q| q.borrow_mut().push_back(ev));
}

fn now() -> f64 {
    js_sys::Date::now()
}

fn random() -> f64 {
    js_sys::Math::random()
}

fn link<T>(f: impl FnOnce(&mut Link) -> T) -> T {
    LINK.with(|l| f(&mut l.borrow_mut()))
}

/// Connect and stay connected. If already doing so, retry now. Progress arrives
/// as `NetEvent`s.
pub fn start() {
    if link(|l| l.started()) {
        run(link(|l| l.restart(now())));
    } else {
        watch_browser();
        run(Some(link(|l| l.connect(now()))));
    }
}

fn run(command: Option<Command>) {
    match command {
        Some(Command::Connect) => {
            drop_socket();
            if open_socket().is_none() {
                link(|l| l.lost(now(), random()));
                push(NetEvent::Closed);
            }
        }
        Some(Command::Drop) => drop_socket(),
        None => {}
    }
}

fn open_socket() -> Option<()> {
    let loc = web_sys::window()?.location();
    let secure = loc.protocol().ok()? == "https:";
    let host = loc.host().ok()?;
    let url = format!("{}://{host}/ws", if secure { "wss" } else { "ws" });
    let ws = WebSocket::new(&url).ok()?;

    let on_open = Closure::<dyn FnMut()>::new(|| {
        link(|l| l.opened(now()));
        push(NetEvent::Open);
    });
    let reply = ws.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
        link(|l| l.received(now()));
        if let Some(text) = e.data().as_string()
            && let Ok(msg) = serde_json::from_str(&text)
        {
            // Answer pings here: a hidden tab stops drawing frames, so the
            // game loop cannot be relied on to do it.
            if matches!(msg, ServerMsg::Ping) {
                if let Ok(pong) = serde_json::to_string(&ClientMsg::Pong) {
                    let _ = reply.send_with_str(&pong);
                }
            } else {
                push(NetEvent::Msg(msg));
            }
        }
    });
    let on_close = Closure::<dyn FnMut(CloseEvent)>::new(|_| {
        link(|l| l.lost(now(), random()));
        push(NetEvent::Closed);
    });

    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    CONN.with(|c| {
        *c.borrow_mut() = Some(Conn {
            ws,
            _closures: (on_open, on_message, on_close),
        })
    });
    Some(())
}

/// Abandon the current socket, if any, and report it lost.
///
/// Not to be called from the socket's own callbacks, which this frees.
fn drop_socket() {
    if let Some(conn) = CONN.with(|c| c.borrow_mut().take()) {
        conn.ws.set_onopen(None);
        conn.ws.set_onclose(None);
        conn.ws.set_onmessage(None);
        let _ = conn.ws.close();
        push(NetEvent::Closed);
    }
}

/// Install the timer and the browser event handlers, once.
fn watch_browser() {
    let Some(window) = web_sys::window() else {
        return;
    };
    if window
        .location()
        .search()
        .is_ok_and(|q| q.contains("chaos=1"))
    {
        schedule_chaos();
    }

    let tick = Closure::<dyn FnMut()>::new(tick);
    let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
        tick.as_ref().unchecked_ref(),
        TICK_MS,
    );
    tick.forget();

    let online = Closure::<dyn FnMut()>::new(|| run(link(|l| l.restart(now()))));
    let _ = window.add_event_listener_with_callback("online", online.as_ref().unchecked_ref());
    online.forget();

    // Back from the back/forward cache, where sockets do not survive.
    let shown = Closure::<dyn FnMut(PageTransitionEvent)>::new(|e: PageTransitionEvent| {
        if e.persisted() {
            run(link(|l| l.restart(now())));
        }
    });
    let _ = window.add_event_listener_with_callback("pageshow", shown.as_ref().unchecked_ref());
    shown.forget();

    if let Some(document) = window.document() {
        let seen = document.clone();
        let visibility = Closure::<dyn FnMut()>::new(move || {
            if !seen.hidden() {
                run(link(|l| l.visible(now())));
            }
        });
        let _ = document.add_event_listener_with_callback(
            "visibilitychange",
            visibility.as_ref().unchecked_ref(),
        );
        visibility.forget();
    }
}

fn tick() {
    if CHAOS.get().is_some_and(|at| now() >= at) {
        schedule_chaos();
        link(|l| l.lost(now(), random()));
        drop_socket();
    }
    run(link(|l| l.tick(now(), random())));
}

/// Cut the connection again in 20 to 60 seconds.
fn schedule_chaos() {
    CHAOS.set(Some(now() + 20_000.0 + 40_000.0 * random()));
}

/// Send if connected. Anything else is dropped: the server's next snapshot is
/// the truth, and the player can act again.
pub fn send(msg: &ClientMsg) {
    CONN.with(|c| {
        if let (Some(conn), Ok(text)) = (c.borrow().as_ref(), serde_json::to_string(msg))
            && conn.ws.ready_state() == WebSocket::OPEN
        {
            let _ = conn.ws.send_with_str(&text);
        }
    });
}

pub fn poll() -> Vec<NetEvent> {
    INBOX.with(|q| q.borrow_mut().drain(..).collect())
}
