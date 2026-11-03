mod hub;

use axum::{
    Router,
    body::Body,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use hub::Hub;
use ludo_core::protocol::{ClientMsg, ServerMsg};
use rust_embed::RustEmbed;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tower_http::compression::CompressionLayer;

/// The trunk build output of the `ludo-game` crate, baked into the binary.
#[derive(RustEmbed)]
#[folder = "../ludo-game/dist"]
struct Game;

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

#[tokio::main]
async fn main() {
    let addr = std::env::var("LUDO_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let turn_ms: u64 = std::env::var("LUDO_TURN_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    let hub = Hub::new(Duration::from_millis(turn_ms));
    let ticker = hub.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            ticker.tick();
        }
    });
    let app = Router::new()
        .route("/ws", get(ws_upgrade))
        .fallback(get(serve_static))
        .layer(CompressionLayer::new())
        .with_state(hub.clone());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind address");
    println!("Ludo server listening on http://{addr}");
    axum::serve(listener, app).await.expect("server error");
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(hub): State<Hub>) -> Response {
    ws.on_upgrade(move |socket| connection(socket, hub))
}

async fn connection(socket: WebSocket, hub: Hub) {
    let conn = NEXT_CONN.fetch_add(1, Ordering::Relaxed);
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMsg>();

    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let text = serde_json::to_string(&msg).expect("serialize ServerMsg");
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(frame)) = stream.next().await {
        let text = match frame {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(msg) = serde_json::from_str::<ClientMsg>(text.as_str()) else {
            continue;
        };
        match msg {
            ClientMsg::Hello {
                client_id,
                name,
                color,
            } => hub.hello(conn, tx.clone(), client_id, name, color),
            ClientMsg::CreateRoom { name, players } => hub.create_room(conn, &name, players),
            ClientMsg::JoinRoom { id } => hub.join_room(conn, id),
            ClientMsg::SetColor { color } => hub.set_color(conn, color),
            ClientMsg::StartGame => hub.start(conn),
            ClientMsg::LeaveRoom => hub.leave(conn),
            ClientMsg::Roll => {
                let value = rand::Rng::random_range(&mut rand::rng(), 1..=6);
                hub.act(conn, |g| g.roll(value));
            }
            ClientMsg::Play { token } => hub.act(conn, |g| g.play(token)),
        }
    }

    hub.disconnect(conn);
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
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(file.data.into_owned()))
                .unwrap()
        }
        None if path == "index.html" => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Game not built. Run `make` to build the game and the server.",
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
