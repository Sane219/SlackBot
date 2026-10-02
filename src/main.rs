mod config;
mod domain;
mod llm;
mod evidence;
mod github;
mod slack;
mod secrets;
mod store;

use axum::{
    http::header,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use config::ServerConfig;
use serde::Serialize;

/// Static assets, embedded in the binary at compile time.
///
/// Embedded rather than read from disk so `cargo run` works from any working
/// directory and the binary needs nothing beside it.
const INDEX_HTML: &str = include_str!("web/index.html");
const APP_CSS: &str = include_str!("web/app.css");

fn assets() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.css", get(css))
}

async fn index() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], INDEX_HTML)
}

async fn css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], APP_CSS)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[tokio::main]
async fn main() {
    let config = ServerConfig::from_env();
    let addr = config.bind_addr();

    if let Err(reason) = config::ensure_loopback(addr) {
        eprintln!("{reason}");
        std::process::exit(1);
    }

    let app = Router::new()
        .route("/api/health", get(health))
        .merge(assets());

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("could not bind {addr}: {err}");
            std::process::exit(1);
        }
    };

    let url = config.url();
    println!("slackbot listening on {url}");
    println!("press ctrl-c to stop");

    if let Err(err) = open::that(&url) {
        // Not fatal: the URL is printed, and a headless or remote machine has no
        // browser to open anyway.
        eprintln!("could not open a browser automatically: {err}");
    }

    if let Err(err) = axum::serve(listener, app).await {
        eprintln!("server error: {err}");
    }
}