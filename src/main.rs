mod config;

use axum::{routing::get, Json, Router};
use config::ServerConfig;
use serde::Serialize;

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

    let app = Router::new().route("/api/health", get(health));

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("could not bind {addr}: {err}");
            std::process::exit(1);
        }
    };

    println!("slackbot listening on {}", config.url());
    println!("press ctrl-c to stop");

    if let Err(err) = axum::serve(listener, app).await {
        eprintln!("server error: {err}");
    }
}