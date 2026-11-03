use axum::{
    Router,
    body::Body,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use rust_embed::RustEmbed;

/// The trunk build output of the `ludo-game` crate, baked into the binary.
#[derive(RustEmbed)]
#[folder = "../ludo-game/dist"]
struct Game;

#[tokio::main]
async fn main() {
    let addr = std::env::var("LUDO_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let app = Router::new().fallback(get(serve));
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind address");
    println!("Ludo server listening on http://{addr}");
    axum::serve(listener, app).await.expect("server error");
}

async fn serve(uri: Uri) -> Response {
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
            "Game not built. Run `./build.sh` (trunk build --release in ludo-game), then rebuild the server.",
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
