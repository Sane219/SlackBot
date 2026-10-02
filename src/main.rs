mod config;
mod domain;
mod evidence;
mod github;
mod llm;
mod routes;
mod scheduler;
mod secrets;
mod slack;
mod store;

use std::sync::Arc;

use config::ServerConfig;
use llm::{LlmClient, LlmConfig};
use routes::AppState;
use scheduler::{FireRunner, LiveSource};
use secrets::{KeychainStore, PresentCredentials, SecretKind};

/// How often the scheduler looks for work. Coarse enough to be unnoticeable, fine
/// enough that a due Job fires close to on time.
const TICK: std::time::Duration = std::time::Duration::from_secs(20);

/// Static assets, embedded at compile time.
///
/// Embedded rather than read from disk so `cargo run` works from any working directory
/// and the binary needs nothing beside it.
const INDEX_HTML: &str = include_str!("web/index.html");
const APP_CSS: &str = include_str!("web/app.css");
const APP_JS: &str = include_str!("web/app.js");

fn assets() -> axum::Router {
    use axum::{http::header, response::IntoResponse, routing::get};
    axum::Router::new()
        .route("/", get(|| async {
            ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], INDEX_HTML)
        }))
        .route("/app.css", get(|| async {
            ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], APP_CSS)
        }))
        .route("/app.js", get(|| async {
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], APP_JS)
        }))
}

/// Build the LLM client from the saved settings and the keychain, if both exist.
fn build_llm(secrets: &dyn secrets::SecretStore) -> Option<Arc<LlmClient>> {
    let settings = config::read_llm_settings(&config::settings_path())?;
    let key = secrets.get(SecretKind::LlmApiKey).ok()?;
    Some(Arc::new(LlmClient::new(LlmConfig::new(
        settings.base_url,
        settings.model,
        key,
    ))))
}

/// Build the Slack client when both halves of the session are stored.
///
/// A half-configured session is not built at all: it would fail every call with an error
/// that looks like an expired cookie rather than a missing field.
///
/// The user id comes from the stored identity, which setup captures by calling
/// `auth.test`. Without it, history filtering would match nothing and every day would
/// look empty — which is the failure mode that looks like a working tool.
fn build_slack(secrets: &dyn secrets::SecretStore) -> Option<Arc<slack::SlackClient>> {
    let token = secrets.get(SecretKind::SlackToken).ok()?;
    let cookie = secrets.get(SecretKind::SlackCookie).ok()?;
    let user_id = config::read_slack_identity(&config::settings_path())?;
    Some(Arc::new(slack::SlackClient::new(token, cookie, user_id)))
}

/// Build the GitHub client when a token is stored.
fn build_github(secrets: &dyn secrets::SecretStore) -> Option<Arc<github::GithubClient>> {
    let token = secrets.get(SecretKind::GithubToken).ok()?;
    let login = config::read_github_login(&config::settings_path())?;
    Some(Arc::new(github::GithubClient::new(token, login)))
}

#[tokio::main]
async fn main() {
    let config = ServerConfig::from_env();
    let addr = config.bind_addr();

    if let Err(reason) = config::ensure_loopback(addr) {
        eprintln!("{reason}");
        std::process::exit(1);
    }

    let keychain = Arc::new(KeychainStore::default());

    let conn = match store::open(&config::db_path()) {
        Ok(conn) => Arc::new(std::sync::Mutex::new(conn)),
        Err(err) => {
            eprintln!("could not open the database at {}: {err}", config::db_path().display());
            std::process::exit(1);
        }
    };

    // A Fire whose time passed while this process was down is recorded now, and never
    // retro-fired (ADR-0007).
    {
        let guard = conn.lock().unwrap_or_else(|e| e.into_inner());
        match scheduler::mark_missed(&guard, domain::now()) {
            Ok(0) => {}
            Ok(n) => println!("{n} missed fire(s) recorded; they will not be sent"),
            Err(err) => eprintln!("could not record missed fires: {err}"),
        }
    }

    let present = PresentCredentials::probe(keychain.as_ref());
    if !present.collection_ready() {
        println!("slackbot is not configured yet — open the UI to set it up");
    }

    let llm = build_llm(keychain.as_ref());
    let slack = build_slack(keychain.as_ref());
    let github = build_github(keychain.as_ref());

    let source = Arc::new(LiveSource {
        slack: slack.as_ref().map(|s| (**s).clone()),
        github: github.as_ref().map(|g| (**g).clone()),
        token_budget: 12_000,
    });

    let runner = Arc::new(FireRunner {
        source,
        llm: llm.clone().unwrap_or_else(|| {
            // A placeholder so the type is satisfied; every path that uses it checks
            // `llm.is_some()` first and refuses with a clear message.
            Arc::new(LlmClient::new(LlmConfig::new(
                "http://127.0.0.1:0",
                "unconfigured",
                secrets::Secret::new(""),
            )))
        }),
    });

    // The scheduler: one task, ticking on the same database the handlers use.
    let scheduler_conn = conn.clone();
    let scheduler_runner = runner.clone();
    tokio::spawn(async move {
        loop {
            // The guard is dropped before the sleep. Holding it across an await would make
            // this task's future non-Send and would block every HTTP request for the whole
            // tick, which includes the model's network call.
            if let Err(err) = scheduler::tick(&scheduler_runner, &scheduler_conn, domain::now()).await
            {
                eprintln!("scheduler tick: {err}");
            }
            tokio::time::sleep(TICK).await;
        }
    });

    let app = routes::router(AppState {
        conn,
        secrets: keychain,
        llm,
        runner: Some(runner),
        slack,
    })
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
        // Not fatal: the URL is printed, and a headless machine has no browser anyway.
        eprintln!("could not open a browser automatically: {err}");
    }

    if let Err(err) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        eprintln!("server error: {err}");
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
