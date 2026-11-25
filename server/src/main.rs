mod hub;

use axum::{
    Router,
    body::Body,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use hub::{Hello, Hub, Timing};
use ludo_core::protocol::{ClientMsg, ServerMsg};
use rust_embed::RustEmbed;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tower_http::compression::CompressionLayer;
use tracing::info;

/// The trunk build output of the `ludo-game` crate, baked into the binary.
#[derive(RustEmbed)]
#[folder = "../ludo-game/dist"]
struct Game;

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
struct Heartbeat {
    /// How often each client is pinged.
    ping: Duration,
    /// A client silent for this long is taken to be gone.
    dead: Duration,
}

#[derive(Clone)]
struct App {
    hub: Hub,
    heartbeat: Heartbeat,
}

fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_ms(name: &str, default: Duration) -> Duration {
    Duration::from_millis(env_number(name, default.as_millis() as u64))
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("LUDO_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".into());
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let default = Timing::default();
    let timing = Timing {
        turn: env_ms("LUDO_TURN_MS", default.turn),
        away_turn: env_ms("LUDO_AWAY_TURN_MS", default.away_turn),
        idle_timeouts: env_number("LUDO_IDLE_TIMEOUTS", default.idle_timeouts.into())
            .clamp(1, u8::MAX.into()) as u8,
        room_grace: env_ms("LUDO_ROOM_GRACE_MS", default.room_grace),
        lobby_grace: env_ms("LUDO_LOBBY_GRACE_MS", default.lobby_grace),
    };
    let heartbeat = Heartbeat {
        ping: env_ms("LUDO_PING_MS", Duration::from_secs(15)).max(Duration::from_millis(50)),
        dead: env_ms("LUDO_DEAD_MS", Duration::from_secs(45)),
    };
    let hub = Hub::new(timing);
    let ticker = hub.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            ticker.tick(Instant::now());
        }
    });
    let app = Router::new()
        .route("/ws", get(ws_upgrade))
        .fallback(get(serve_static))
        .layer(CompressionLayer::new())
        .with_state(App {
            hub: hub.clone(),
            heartbeat,
        });
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind address");
    info!("Ludo server listening on http://{addr}");
    // Open sockets would hold up a graceful shutdown for ever, so just warn
    // them and give the warning a moment to go out.
    tokio::select! {
        served = axum::serve(listener, app) => served.expect("server error"),
        () = stop_requested() => {
            info!("shutting down");
            hub.announce_restart();
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

/// Resolves on Ctrl-C or, as `docker stop` sends, SIGTERM.
async fn stop_requested() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate()).expect("watch SIGTERM");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate.recv() => {}
    }
}

async fn ws_upgrade(ws: WebSocketUpgrade, headers: HeaderMap, State(app): State<App>) -> Response {
    // The visitor's address when behind Cloudflare.
    let ip = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_owned();
    ws.on_upgrade(move |socket| connection(socket, app, ip))
}

async fn connection(socket: WebSocket, app: App, ip: String) {
    let App { hub, heartbeat } = app;
    let conn = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
    info!(conn, %ip, "connected");
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMsg>();

    let writer = tokio::spawn(async move {
        let mut ping =
            tokio::time::interval_at(tokio::time::Instant::now() + heartbeat.ping, heartbeat.ping);
        loop {
            let msg = tokio::select! {
                msg = rx.recv() => match msg {
                    Some(msg) => msg,
                    None => break,
                },
                _ = ping.tick() => ServerMsg::Ping,
            };
            let text = serde_json::to_string(&msg).expect("serialize ServerMsg");
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // Until a client says it does not answer pings, silence means it is gone.
    let mut expect_pongs = true;
    let reason = loop {
        let next = if expect_pongs {
            match tokio::time::timeout(heartbeat.dead, stream.next()).await {
                Ok(next) => next,
                Err(_) => break "timed out",
            }
        } else {
            stream.next().await
        };
        let text = match next {
            Some(Ok(Message::Text(t))) => t,
            Some(Ok(Message::Close(_))) | None => break "closed",
            Some(Ok(_)) => continue,
            Some(Err(_)) => break "connection error",
        };
        let Ok(msg) = serde_json::from_str::<ClientMsg>(text.as_str()) else {
            continue;
        };
        let now = Instant::now();
        match msg {
            ClientMsg::Hello {
                client_id,
                name,
                color,
                heartbeat,
                room,
            } => {
                expect_pongs = heartbeat;
                let hello = Hello {
                    client_id,
                    name,
                    color,
                    room,
                };
                hub.hello(conn, tx.clone(), hello, now);
            }
            ClientMsg::Pong => {}
            ClientMsg::CreateRoom { name, players } => hub.create_room(conn, &name, players, now),
            ClientMsg::JoinRoom { id } => hub.join_room(conn, id, now),
            ClientMsg::SetColor { color } => hub.set_color(conn, color, now),
            ClientMsg::StartGame => hub.start(conn, now),
            ClientMsg::LeaveRoom => hub.leave(conn, now),
            ClientMsg::Roll { rev } => {
                let value = rand::Rng::random_range(&mut rand::rng(), 1..=6);
                hub.act(conn, rev, now, |g| g.roll(value));
            }
            ClientMsg::Play { token, rev } => hub.act(conn, rev, now, |g| g.play(token)),
        }
    };

    info!(conn, reason, "disconnected");
    hub.disconnect(conn, Instant::now());
    writer.abort();
}

async fn serve_static(uri: Uri) -> Response {
    let path = match uri.path().trim_start_matches('/') {
        "" => "index.html",
        p => p,
    };
    match Game::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            let mut response = Response::builder().header(header::CONTENT_TYPE, mime.as_ref());
            // The page names the current build, so it must not be cached: a
            // client reloading to update has to get the new one.
            if path == "index.html" {
                response = response.header(header::CACHE_CONTROL, "no-cache");
            }
            response.body(Body::from(file.data.into_owned())).unwrap()
        }
        None if path == "index.html" => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Game not built. Run `make` to build the game and the server.",
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
