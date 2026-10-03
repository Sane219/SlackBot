//! The HTTP surface.
//!
//! The load-bearing fact about this module: `chat.postMessage` is called from exactly one
//! place, `approve`. Everything else reads or drafts. ADR-0001 makes that a property of
//! the code rather than a setting, and a test asserts no second caller exists.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::{now, Draft, Fire, Job};
use crate::evidence::NO_SIGNAL_TEXT;
use crate::llm::{LlmClient, LlmError, PlannedJob};
use crate::scheduler::FireRunner;
use crate::secrets::{PresentCredentials, Secret, SecretKind, SecretStore};
use crate::store::{self, StoreError};

/// Everything a handler needs. Behind Arc so axum's state is cheap to clone.
#[derive(Clone)]
pub struct AppState {
    /// A Mutex because `rusqlite::Connection` is Send but not Sync, and axum's state
    /// must be both. Every hold is a short query, so the contention is negligible; a
    /// connection pool would be the answer if a query ever got long.
    pub conn: Arc<std::sync::Mutex<rusqlite::Connection>>,
    pub secrets: Arc<dyn SecretStore>,
    pub llm: Option<Arc<LlmClient>>,
    pub runner: Option<Arc<FireRunner>>,
    /// Present only when both Slack credentials are stored. Approve is the sole caller
    /// of its `post` method.
    pub slack: Option<Arc<crate::slack::SlackClient>>,
}

type ApiError = (StatusCode, String);

/// Reject a request that did not come from this tool's own page.
///
/// The server binds loopback, which is not an authorisation boundary: any page in any
/// browser can reach `127.0.0.1`. A cross-origin write could be triggered by a page the
/// user simply opened.
fn reject_cross_origin(headers: &axum::http::HeaderMap) -> Result<(), ApiError> {
    // Modern browsers always send Sec-Fetch-Site on a same-origin request.
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        if site != "same-origin" && site != "none" {
            return Err((
                StatusCode::FORBIDDEN,
                "cross-origin request refused: this tool only accepts its own page".into(),
            ));
        }
    }

    // Fall back to Origin for a browser that omits Sec-Fetch-*.
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        let host = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))
            .unwrap_or(origin);
        // Loopback in any spelling: 127.0.0.1, localhost, or the IPv6 form.
        let is_local = host.starts_with("127.")
            || host.starts_with("[::1]")
            || host.starts_with("localhost")
            || host.starts_with("::1");
        if !is_local {
            return Err((
                StatusCode::FORBIDDEN,
                "cross-origin request refused: this tool only accepts its own page".into(),
            ));
        }
    }

    Ok(())
}

/// Refuse a Draft with no text.
///
/// An empty body would post an empty message, which is indistinguishable from a bug in
/// the channel and cannot be explained afterwards.
fn reject_empty_draft(text: &str) -> Result<(), ApiError> {
    if text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "a draft cannot be empty".into()));
    }
    Ok(())
}

/// Require a JSON content type on a write.
///
/// A cross-origin HTML form can send `application/x-www-form-urlencoded`,
/// `text/plain`, and `multipart/form-data` without a preflight, but not
/// `application/json`. This is the cheap half of the same boundary.
fn require_json_content_type(headers: &axum::http::HeaderMap) -> Result<(), ApiError> {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.starts_with("application/json") {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "this endpoint accepts application/json only".into(),
        ));
    }
    Ok(())
}

/// Take the database lock, recovering from a poisoned mutex.
///
/// A panic while holding the lock poisons it. Refusing to serve afterwards would mean one
/// panic takes the tool down for the rest of the session, so the guard is taken and the
/// connection used anyway.
fn lock(state: &AppState) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
    lock_conn(&state.conn)
}

/// The same, for the handful of places that hold a connection rather than the whole
/// state. `deliver` is one: it is called both from an HTTP handler and from the
/// scheduler's auto-send branch, and only the handler has an `AppState`.
fn lock_conn(
    conn: &std::sync::Mutex<rusqlite::Connection>,
) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
    conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl From<StoreError> for ApiError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::JobNotFound(_) | StoreError::DraftNotFound(_) => {
                (StatusCode::NOT_FOUND, err.to_string())
            }
            // A database failure is ours, not the user's; the message is deliberately
            // terse because it will be shown in a browser.
            other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        }
    }
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
    configured: PresentCredentials,
    /// How many Drafts are waiting. The one number a user checks on opening the page.
    inbox: usize,
}

async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        configured: PresentCredentials::probe(state.secrets.as_ref()),
        inbox: store::list_unapproved(&lock(&state))
            .map(|d| d.len())
            .unwrap_or(0),
    })
}

// ── Setup ──────────────────────────────────────────────────────────────────

/// Which credentials exist, and nothing else. Never their values.
#[derive(Serialize)]
struct SetupStatus {
    present: PresentCredentials,
    slack_ready: bool,
    collection_ready: bool,
    /// ADR-0010. Read here rather than through a separate endpoint so the Setup board
    /// has one request, and so the switch cannot show a state the server disagrees with.
    auto_send: bool,
}

async fn setup_status(State(state): State<AppState>) -> Json<SetupStatus> {
    let present = PresentCredentials::probe(state.secrets.as_ref());
    Json(SetupStatus {
        slack_ready: present.slack_ready(),
        collection_ready: present.collection_ready(),
        auto_send: crate::config::read_auto_send(&crate::config::settings_path()),
        present,
    })
}

/// One credential row. `value` is write-only: it is stored and never returned.
#[derive(Deserialize)]
struct SaveCredential {
    kind: SecretKind,
    value: String,
}

#[derive(Serialize)]
struct SaveResult {
    ok: bool,
    present: PresentCredentials,
    /// A non-fatal note, e.g. the model was not verified.
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

async fn save_credential(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<SaveCredential>,
) -> Result<Json<SaveResult>, ApiError> {
    reject_cross_origin(&headers)?;
    let value = body.value.trim().to_string();

    // An empty value is rejected rather than stored: a blank credential fails later, at
    // a Fire, where the cause is much harder to see.
    if value.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "a credential value is required".into(),
        ));
    }

    // The Slack cookie is passed exactly as the browser sent it. Decoding it is the most
    // likely way to break an otherwise-correct setup (ADR-0008).
    // The message says "without the `d=` prefix" but the old guard only checked for a
    // space, so `d=xoxd-…` was accepted, stored, and sent as `Cookie: d=d=xoxd-…`.
    // ADR-0008 calls a mangled cookie the single most likely way to break a setup that
    // otherwise looks correct.
    if body.kind == SecretKind::SlackCookie {
        if value.contains(' ') {
            return Err((
                StatusCode::BAD_REQUEST,
                "paste the cookie value only, with no spaces".into(),
            ));
        }
        if let Some(rest) = value.strip_prefix("d=") {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "paste the value only, without the `d=` prefix (it starts with {})",
                    rest.chars().take(8).collect::<String>()
                ),
            ));
        }
    }

    state
        .secrets
        .set(body.kind, &Secret::new(value))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let present = PresentCredentials::probe(state.secrets.as_ref());
    Ok(Json(SaveResult {
        ok: true,
        present,
        note: None,
    }))
}

/// Verify one credential and capture the identity it implies.
///
/// The identities are stored because they are needed to read anything: the Slack `U…` id
/// filters history to the user's own messages, and the GitHub login scopes discovery.
/// Neither is a secret, so neither goes in the keychain.
#[derive(Serialize)]
struct VerifyResult {
    ok: bool,
    /// `llm`, `slack`, or `github`.
    kind: &'static str,
    /// The verified identity, where the service reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    identity: Option<String>,
    /// A short, actionable reason when `ok` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

async fn verify_credential(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<SecretKindBody>,
) -> Result<Json<VerifyResult>, ApiError> {
    reject_cross_origin(&headers)?;
    require_json_content_type(&headers)?;
    let settings = crate::config::settings_path();
    let fail = |kind: &'static str, reason: String| VerifyResult {
        ok: false,
        kind,
        identity: None,
        reason: Some(reason),
    };

    let result = match body.kind {
        SecretKind::SlackToken | SecretKind::SlackCookie => {
            // Both halves are needed: this call is the one that catches a mangled cookie.
            let (Ok(token), Ok(cookie)) = (
                state.secrets.get(SecretKind::SlackToken),
                state.secrets.get(SecretKind::SlackCookie),
            ) else {
                return Ok(Json(fail(
                    "slack",
                    "both the token and the cookie are needed".into(),
                )));
            };

            let client = crate::slack::SlackClient::new(
                token,
                cookie,
                std::env::var("SLACKBOT_SLACK_USER_ID").unwrap_or_default(),
            );
            match client.verify().await {
                Ok(user_id) => {
                    let _ = crate::config::write_slack_identity(&settings, &user_id);
                    VerifyResult {
                        ok: true,
                        kind: "slack",
                        identity: Some(user_id),
                        reason: None,
                    }
                }
                Err(err) => fail("slack", err.to_string()),
            }
        }
        SecretKind::GithubToken => {
            let Ok(token) = state.secrets.get(SecretKind::GithubToken) else {
                return Ok(Json(fail("github", "no token stored yet".into())));
            };
            let client = crate::github::GithubClient::new(
                token,
                std::env::var("GITHUB_LOGIN").unwrap_or_default(),
            );
            match client.whoami().await {
                Ok(login) => {
                    let _ = crate::config::write_github_login(&settings, &login);
                    VerifyResult {
                        ok: true,
                        kind: "github",
                        identity: Some(login),
                        reason: None,
                    }
                }
                Err(err) => fail("github", err.to_string()),
            }
        }
        SecretKind::LlmApiKey => {
            let Some(client) = state.llm.as_ref() else {
                return Ok(Json(fail("llm", "set the endpoint and model first".into())));
            };
            match client.probe().await {
                Ok(models) => VerifyResult {
                    ok: true,
                    kind: "llm",
                    identity: Some(format!("{} model(s) available", models.len())),
                    reason: None,
                },
                Err(err) => fail("llm", err.to_string()),
            }
        }
    };

    Ok(Json(result))
}

#[derive(Deserialize)]
struct SecretKindBody {
    kind: SecretKind,
}

/// The LLM endpoint and model, which are not secrets and so live in a small side file
/// next to the database.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub struct LlmSettings {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Deserialize)]
struct SaveLlm {
    base_url: String,
    model: String,
}

async fn save_llm(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<SaveLlm>,
) -> Result<Json<SaveResult>, ApiError> {
    reject_cross_origin(&headers)?;
    let base_url = body.base_url.trim().to_string();
    let model = body.model.trim().to_string();

    if base_url.is_empty() || model.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "an endpoint and a model name are both required".into(),
        ));
    }
    // A bad endpoint must fail here, not at 14:30 with a window to cover.
    let probe = crate::llm::LlmConfig::new(
        base_url.clone(),
        model.clone(),
        state
            .secrets
            .get(SecretKind::LlmApiKey)
            .unwrap_or(Secret::new("")),
    );
    let client = LlmClient::new(probe);

    let mut note = None;
    match client.probe().await {
        Ok(models) => {
            if !models.is_empty() && !models.contains(&model) {
                // Not fatal: some endpoints list models inconsistently. Worth saying.
                note = Some(format!(
                    "the endpoint answered but did not list `{model}`; it will be used anyway"
                ));
            }
        }
        Err(err) => {
            return Err((
                StatusCode::BAD_GATEWAY,
                format!("could not reach {base_url}: {err}"),
            ))
        }
    }

    crate::config::write_llm_settings(
        &crate::config::settings_path(),
        &LlmSettings { base_url, model },
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(SaveResult {
        ok: true,
        present: PresentCredentials::probe(state.secrets.as_ref()),
        note,
    }))
}

async fn llm_settings(State(state): State<AppState>) -> Json<LlmSettings> {
    let _ = &state;
    Json(crate::config::read_llm_settings(&crate::config::settings_path()).unwrap_or_default())
}

// ── Auto-send ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SetAutoSend {
    enabled: bool,
}

/// Turn unattended posting on or off (ADR-0010).
///
/// A bool and nothing else: no readiness check, no mode object. Turning it on with Slack
/// unconfigured is not a mistake worth guarding against, because `deliver` already fails
/// safely in that case — the Draft is left unclaimed in the Inbox and the Fire shows as
/// failed, which is where a real problem would already be visible.
async fn set_auto_send(
    headers: axum::http::HeaderMap,
    Json(body): Json<SetAutoSend>,
) -> Result<StatusCode, ApiError> {
    reject_cross_origin(&headers)?;
    require_json_content_type(&headers)?;

    crate::config::write_auto_send(&crate::config::settings_path(), body.enabled)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Jobs ───────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct JobsResponse {
    jobs: Vec<Job>,
}

/// The channels the user can post to, resolved from names to Slack's `C…` ids.
///
/// The Plan Role returns names like `#coot-ai`, because that is what a person writes.
/// `chat.postMessage` and `conversations.history` both need ids, so a Job created from a
/// proposal would have failed every read with `channel_not_found` and every post with a
/// 502. This is the only way to turn a name into an id.
#[derive(Serialize)]
struct ChannelView {
    id: String,
    name: String,
}

async fn list_channels(State(state): State<AppState>) -> Result<Json<Vec<ChannelView>>, ApiError> {
    let slack = state.slack.as_ref().ok_or((
        StatusCode::PRECONDITION_FAILED,
        "slack is not configured yet".into(),
    ))?;

    let channels = slack
        .channels()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    Ok(Json(
        channels
            .into_iter()
            .map(|c| ChannelView {
                id: c.id,
                name: c.name,
            })
            .collect(),
    ))
}

/// Resolve a channel name to its id, for a Job created from a plan.
async fn resolve_channel(
    slack: &crate::slack::SlackClient,
    name: &str,
) -> Result<crate::domain::ChannelRef, ApiError> {
    let wanted = name.trim_start_matches('#').to_lowercase();
    let channels = slack
        .channels()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    channels
        .into_iter()
        .find(|c| c.name.to_lowercase() == wanted)
        .map(|c| crate::domain::ChannelRef {
            id: c.id,
            name: c.name,
        })
        .ok_or((
            StatusCode::BAD_REQUEST,
            format!(
                "no channel called #{wanted} that you are a member of. \
                 Slack needs the channel's id, not its name — pick one from the list."
            ),
        ))
}

async fn list_jobs(State(state): State<AppState>) -> Result<Json<JobsResponse>, ApiError> {
    Ok(Json(JobsResponse {
        jobs: store::list_jobs(&lock(&state))?,
    }))
}

#[derive(Deserialize)]
struct PlanRequest {
    /// The user's plain-English description of their routine.
    description: String,
}

#[derive(Serialize)]
struct PlanResponse {
    /// Proposed Jobs. Nothing is scheduled until the user posts these back.
    jobs: Vec<PlannedJob>,
}

async fn plan(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<PlanRequest>,
) -> Result<Json<PlanResponse>, ApiError> {
    reject_cross_origin(&headers)?;
    let description = body.description.trim().to_string();
    if description.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "describe your routine in a sentence or two".into(),
        ));
    }

    let llm = state.llm.as_ref().ok_or((
        StatusCode::PRECONDITION_FAILED,
        "no model is configured yet".into(),
    ))?;

    match llm.plan(&description).await {
        Ok(jobs) => Ok(Json(PlanResponse { jobs })),
        // An unusable proposal is shown raw. It is never repaired into a working
        // schedule the user did not choose.
        Err(LlmError::UnusablePlan { detail, raw }) => Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{detail}\n\nthe model said:\n{raw}"),
        )),
        Err(err) => Err((StatusCode::BAD_GATEWAY, err.to_string())),
    }
}

#[derive(Deserialize)]
struct CreateJob {
    name: String,
    at: String,
    tz: String,
    /// Either a Slack `C…` id or a `#name`, which the Plan Role produces.
    channel_id: String,
    /// Optional: derived from the name when a name was given, so a caller that knows
    /// only the channel does not have to send a field the server can work out.
    #[serde(default)]
    channel_name: String,
    context: String,
    // Every optional field defaults. A client that sends only what it means to set is the
    // normal case, and requiring `previous_day: false` to be spelled out would turn a
    // partial body into a 422 before any of the real validation ran.
    #[serde(default)]
    since_at: Option<String>,
    #[serde(default)]
    previous_day: bool,
    #[serde(default)]
    lookback_hours: Option<i64>,
    #[serde(default)]
    prompt: String,
}

async fn create_job(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<CreateJob>,
) -> Result<Json<Job>, ApiError> {
    reject_cross_origin(&headers)?;
    if body.name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "a job needs a name".into()));
    }
    if body.at.parse::<chrono::NaiveTime>().is_err() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{:?} is not a HH:MM time", body.at),
        ));
    }
    if body.tz.parse::<chrono_tz::Tz>().is_err() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{:?} is not a known IANA timezone", body.tz),
        ));
    }

    let tz = body.tz.clone();
    let context_window = match body.context.as_str() {
        "since" => {
            let at = body.since_at.ok_or((
                StatusCode::BAD_REQUEST,
                "a since context needs a time".into(),
            ))?;
            if at.parse::<chrono::NaiveTime>().is_err() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("{at:?} is not a HH:MM time"),
                ));
            }
            crate::domain::ContextWindow::Since {
                at,
                previous_day: body.previous_day,
                tz,
            }
        }
        _ => crate::domain::ContextWindow::Lookback {
            hours: body.lookback_hours.unwrap_or(8).clamp(1, 72),
        },
    };

    // Accept either a Slack `C…` id or a `#name`, because the Plan Role produces the
    // latter and a human types the former. Posting a name where an id is expected fails
    // at collection time with `channel_not_found`, which reads as a broken tool.
    let channel = if body.channel_id.starts_with('C') {
        crate::domain::ChannelRef {
            id: body.channel_id,
            name: body.channel_name,
        }
    } else {
        let slack = state.slack.as_ref().ok_or((
            StatusCode::PRECONDITION_FAILED,
            "set up Slack first, so a channel name can be resolved to its id".into(),
        ))?;
        resolve_channel(slack, &body.channel_id).await?
    };

    let job = Job {
        id: 0,
        name: body.name.trim().into(),
        schedule: crate::domain::Schedule::Daily {
            at: body.at,
            tz: body.tz,
        },
        channel,
        context_window,
        prompt_template: body.prompt,
        enabled: true,
        last_fired_at: None,
    };

    let id = store::insert_job(&lock(&state), &job)?;
    Ok(Json(store::get_job(&lock(&state), id)?))
}

#[derive(Deserialize)]
struct UpdateJob {
    name: Option<String>,
    at: Option<String>,
    tz: Option<String>,
    enabled: Option<bool>,
    prompt: Option<String>,
}

async fn update_job(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: axum::http::HeaderMap,
    Json(body): Json<UpdateJob>,
) -> Result<Json<Job>, ApiError> {
    reject_cross_origin(&headers)?;
    let mut job = store::get_job(&lock(&state), id)?;

    if let Some(name) = body.name {
        job.name = name;
    }
    if let Some(at) = body.at {
        if at.parse::<chrono::NaiveTime>().is_err() {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("{at:?} is not a HH:MM time"),
            ));
        }
        let crate::domain::Schedule::Daily { at: existing, .. } = &mut job.schedule;
        *existing = at;
    }
    if let Some(tz) = body.tz {
        if tz.parse::<chrono_tz::Tz>().is_err() {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("{tz:?} is not a known IANA timezone"),
            ));
        }
        let crate::domain::Schedule::Daily { tz: existing, .. } = &mut job.schedule;
        *existing = tz;
    }
    if let Some(enabled) = body.enabled {
        job.enabled = enabled;
    }
    if let Some(prompt) = body.prompt {
        job.prompt_template = prompt;
    }

    store::update_job(&lock(&state), &job)?;
    Ok(Json(store::get_job(&lock(&state), id)?))
}

async fn delete_job(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: axum::http::HeaderMap,
) -> Result<StatusCode, ApiError> {
    reject_cross_origin(&headers)?;
    store::delete_job(&lock(&state), id)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Inbox ──────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct InboxResponse {
    drafts: Vec<DraftView>,
    fires: Vec<FireView>,
}

#[derive(Serialize)]
struct DraftView {
    #[serde(flatten)]
    draft: Draft,
    /// The Job's name, so the Inbox does not need a second lookup to read.
    job_name: String,
}

#[derive(Serialize)]
struct FireView {
    #[serde(flatten)]
    fire: Fire,
    job_name: String,
}

async fn inbox(State(state): State<AppState>) -> Result<Json<InboxResponse>, ApiError> {
    let jobs = store::list_jobs(&lock(&state))?;
    let name_of = |id: i64| {
        jobs.iter()
            .find(|j| j.id == id)
            .map(|j| j.name.clone())
            .unwrap_or_else(|| "(deleted job)".into())
    };

    let drafts = store::list_unapproved(&lock(&state))?
        .into_iter()
        .map(|draft| DraftView {
            job_name: name_of(draft.job_id),
            draft,
        })
        .collect();

    let fires = store::list_fires(&lock(&state), 50)?
        .into_iter()
        .map(|fire| FireView {
            job_name: name_of(fire.job_id),
            fire,
        })
        .collect();

    Ok(Json(InboxResponse { drafts, fires }))
}

#[derive(Deserialize)]
struct EditDraft {
    text: String,
}

async fn edit_draft(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: axum::http::HeaderMap,
    Json(body): Json<EditDraft>,
) -> Result<Json<Draft>, ApiError> {
    reject_cross_origin(&headers)?;
    reject_empty_draft(&body.text)?;
    store::update_draft_text(&lock(&state), id, &body.text)?;
    Ok(Json(store::get_draft(&lock(&state), id)?))
}

async fn discard_draft(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: axum::http::HeaderMap,
) -> Result<StatusCode, ApiError> {
    reject_cross_origin(&headers)?;
    store::discard_draft(&lock(&state), id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Re-draft against the **original** window, never the current one.
///
/// ADR-0006: Evidence is not persisted, so this re-fetches. Using the current window
/// would let a Draft's meaning drift just because the user pressed a button late.
async fn regenerate_draft(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Draft>, ApiError> {
    reject_cross_origin(&headers)?;
    let draft = store::get_draft(&lock(&state), id)?;

    // Regenerating an already-posted Draft would rewrite the record of what was sent.
    if draft.approved {
        return Err((
            StatusCode::CONFLICT,
            "this draft has already been sent, so its text is a record of what was posted".into(),
        ));
    }

    let runner = state.runner.as_ref().ok_or((
        StatusCode::PRECONDITION_FAILED,
        "no evidence source is configured".into(),
    ))?;
    let job = store::get_job(&lock(&state), draft.job_id)?;

    let activity = runner
        .source
        .collect(job.clone(), draft.window_from, draft.window_to)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))?;

    let rendered = crate::evidence::render(
        &activity,
        draft.window_from,
        draft.window_to,
        crate::evidence::SourceStatus::both(),
        12_000,
    );

    let text = if rendered.no_signal {
        NO_SIGNAL_TEXT.to_string()
    } else {
        let parent = runner
            .latest_draft_for(&lock(&state), job.id)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let parent_text = match parent {
            Some(pid) if pid != id => store::get_draft(&lock(&state), pid)
                .ok()
                .map(|d| d.text)
                .filter(|t| t != NO_SIGNAL_TEXT),
            _ => None,
        };
        state
            .llm
            .as_ref()
            .ok_or((
                StatusCode::PRECONDITION_FAILED,
                "no model is configured yet".into(),
            ))?
            .draft(&job, &rendered.text, parent_text.as_deref())
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?
    };

    // A regenerate replaces the Draft in place: a second row would leave the user
    // choosing between two versions of the same Fire. The counts go with the text, or
    // the UI would describe the first render while showing the second.
    store::replace_draft(
        &lock(&state),
        id,
        &text,
        &rendered.counts,
        rendered.no_signal,
        rendered.partial,
    )?;
    Ok(Json(store::get_draft(&lock(&state), id)?))
}

/// The outcome of posting a Draft. Returned over HTTP and read by the scheduler's
/// auto-send branch, so its fields are public.
#[derive(Serialize)]
pub struct ApproveResult {
    pub ok: bool,
    /// Slack's message timestamp, so the user can find what was sent.
    pub ts: String,
    /// Set when nothing was sent, which is the case for a gap.
    pub note: Option<String>,
}

/// Post a Draft to Slack. The only function that calls `chat.postMessage`.
///
/// ADR-0001 made this a property of the code rather than a setting. ADR-0010 widened
/// *who* may reach it — a human clicking Approve, or auto-send — but deliberately did not
/// widen *how many places* it is called from. The call site count stays at one, so the
/// guard sequence below cannot be duplicated and drift, and `only_approve_reaches_slack`
/// keeps checking something true.
///
/// Every precondition is evaluated before the claim. A check after it returns without
/// releasing, which strands the Draft as approved-but-unsent; that happened once already
/// when "slack is not configured" came after the claim, and it destroyed a drafted post
/// silently.
pub async fn deliver(
    conn: &std::sync::Mutex<rusqlite::Connection>,
    slack_client: Option<&crate::slack::SlackClient>,
    id: i64,
) -> Result<ApproveResult, ApiError> {
    let draft = store::get_draft(&lock_conn(conn), id)?;

    if draft.approved {
        return Err((
            StatusCode::CONFLICT,
            "this draft has already been approved".into(),
        ));
    }

    // An empty Draft means the model call failed. Approving it would post an empty
    // message, which is indistinguishable from a bug in the channel.
    if draft.text.trim().is_empty() {
        store::discard_draft(&lock_conn(conn), id)?;
        return Err((
            StatusCode::CONFLICT,
            "this draft could not be generated, so there is nothing to send".into(),
        ));
    }

    // A gap has nothing to post. Posting it would put "NO SIGNAL" in a channel, which is
    // worse than nothing.
    if draft.no_signal {
        store::discard_draft(&lock_conn(conn), id)?;
        return Ok(ApproveResult {
            ok: false,
            ts: String::new(),
            note: Some(
                "nothing was collected for that window, so there was nothing to post".into(),
            ),
        });
    }

    // Before the claim. See the note on this function.
    let slack = slack_client.ok_or((
        StatusCode::PRECONDITION_FAILED,
        "slack is not configured yet".into(),
    ))?;

    // Claim before the Slack call. Reading `approved` and *then* posting was not atomic:
    // two concurrent requests both saw false, both posted, and the user got two copies of
    // the same message. The claim is the only step after this that leaves state changed,
    // and the one that can fail — the Slack send — releases it.
    let claimed = store::claim_for_send(&lock_conn(conn), id, now())?;
    if !claimed {
        return Err((
            StatusCode::CONFLICT,
            "this draft is already being sent".into(),
        ));
    }

    let job = store::get_job(&lock_conn(conn), draft.job_id)?;

    match slack.post(&job.channel.id, &draft.text).await {
        Ok(ts) => Ok(ApproveResult {
            ok: true,
            ts,
            note: None,
        }),
        Err(err) => {
            // The send failed, so the Draft goes back in the Inbox. Leaving it claimed
            // would silently swallow a post the user never sent.
            store::release_claim(&lock_conn(conn), id)?;
            Err((StatusCode::BAD_GATEWAY, err.to_string()))
        }
    }
}

async fn approve(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: axum::http::HeaderMap,
) -> Result<Json<ApproveResult>, ApiError> {
    // A page the user happens to be visiting can POST to loopback: DNS-rebinding and
    // cross-origin form posts both reach this server, and a body-less POST needed no
    // token. That would post to a team channel with no human clicking anything. A
    // same-origin check closes it, and it is checked here rather than in `deliver`
    // because auto-send is not an HTTP request and has no headers to check.
    reject_cross_origin(&headers)?;
    require_json_content_type(&headers)?;

    deliver(&state.conn, state.slack.as_deref(), id)
        .await
        .map(Json)
}

/// Fire a Job immediately, for testing the whole path without waiting for the clock.
async fn fire_now(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Draft>, ApiError> {
    let job = store::get_job(&lock(&state), id)?;
    let runner = state.runner.as_ref().ok_or((
        StatusCode::PRECONDITION_FAILED,
        "no evidence source is configured".into(),
    ))?;

    // The lock is held only for the short database step, never across the model's network
    // call: a Fire must not block every other request for the seconds a draft takes.
    // Each guard is dropped at the end of its own statement, so neither survives an await.
    let at = now();
    // Collect with no lock held: the network work must not block other requests.
    let collected = runner.collect_for_fire(&job, at).await;

    // Record, then draft. Each holds the lock for a short step only.
    let fire_id = {
        let guard = lock(&state);
        runner.record(&guard, job.id, at, &collected)
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let Some(fire_id) = fire_id else {
        return Err((
            StatusCode::CONFLICT,
            "this job already fired in the current tick".into(),
        ));
    };

    // The seed is read in a short step, then the model is called with no lock held.
    let seed = {
        let guard = lock(&state);
        runner.seed_text(&guard, job.id)
    }
    .unwrap_or(None);
    let (text, parent_id) = runner.compose_text(&job, &collected, seed).await;

    {
        let guard = lock(&state);
        runner.write_draft(
            &guard,
            &job,
            fire_id,
            collected.rendered,
            collected.from,
            collected.to,
            at,
            text,
            parent_id,
        )
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let latest = {
        let guard = lock(&state);
        store::list_unapproved(&guard)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    latest
        .into_iter()
        .find(|d| d.job_id == id)
        .map(Json)
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "the fire produced no draft".into(),
        ))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/setup", get(setup_status))
        .route("/api/setup/credential", post(save_credential))
        .route("/api/setup/verify", post(verify_credential))
        .route("/api/setup/llm", get(llm_settings).post(save_llm))
        .route("/api/setup/auto-send", post(set_auto_send))
        .route("/api/jobs", get(list_jobs).post(create_job))
        .route("/api/channels", get(list_channels))
        .route(
            "/api/jobs/{id}",
            axum::routing::patch(update_job).delete(delete_job),
        )
        .route("/api/jobs/{id}/fire", post(fire_now))
        .route("/api/plan", post(plan))
        .route("/api/inbox", get(inbox))
        .route(
            "/api/drafts/{id}",
            axum::routing::patch(edit_draft).delete(discard_draft),
        )
        .route("/api/drafts/{id}/regenerate", post(regenerate_draft))
        .route("/api/drafts/{id}/approve", post(approve))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use crate::domain::FireOutcome;

    /// Everything above the test module, so a source-scanning test does not read itself.
    ///
    /// Two earlier attempts used `split("#[cfg(test)]").next()`, which truncated
    /// `scheduler.rs` at line 140 — hundreds of lines above the real code, because that
    /// file gates its test *helper* on `#[cfg(test)]`, not just its test module. The
    /// scheduler's caller was invisible and the test reported one caller where there were
    /// two. Cutting at `mod tests` is what was actually meant.
    fn production(source: &str) -> &str {
        source
            .split("\n#[cfg(test)]\nmod tests")
            .next()
            .unwrap_or(source)
    }

    /// ADR-0001: `chat.postMessage` is reachable from exactly one function. A second
    /// caller would mean something could post without a human clicking Approve.
    #[test]
    fn only_approve_reaches_slack() {
        // The previous version of this test skipped any file whose text merely mentioned
        // `chat.postMessage` — which is `routes.rs`, the only file with a real caller. It
        // therefore asserted nothing about the file that matters, and adding
        // `slack.post(..)` to any other handler would have passed.
        //
        // Match on the *call signature* rather than on the substring `.post(`, which also
        // appears in axum's routing builder (`get(..).post(..)`) and in reqwest.
        // `routes.rs` is checked with this test's own body removed, so the pattern being
        // matched does not match itself.
        let routes_without_tests = include_str!("routes.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default()
            .to_string();

        let sources: [(&str, &str); 5] = [
            ("slack.rs", include_str!("slack.rs")),
            ("scheduler.rs", include_str!("scheduler.rs")),
            ("routes.rs", routes_without_tests.as_str()),
            ("main.rs", include_str!("main.rs")),
            ("github.rs", include_str!("github.rs")),
        ];

        // `SlackClient::post(&self, channel: &str, text: &str)`, so a call is a `.post(`
        // whose first argument is a `&` — routing and reqwest calls are bare paths or
        // method values.
        let mut call_sites: Vec<(&str, usize)> = Vec::new();
        for (name, source) in sources {
            for (index, line) in source.lines().enumerate() {
                // Strip line comments properly: a doc comment mentioning `.post(&`
                // must not count as a call site.
                let code = match line.find("//") {
                    Some(i) if !line[..i].contains('"') => line[..i].trim(),
                    _ => line.trim(),
                };
                if code.starts_with("//") || code.is_empty() {
                    continue;
                }
                if code.contains("chat.postMessage") {
                    continue;
                }
                // The client building its own request is the definition, not a caller.
                if name == "slack.rs" && code.contains("format!(\"{}/chat.postMessage\"") {
                    continue;
                }
                if code.contains(".post(&") {
                    call_sites.push((name, index + 1));
                }
            }
        }

        assert_eq!(
            call_sites.len(),
            1,
            "SlackClient::post must be called from exactly one place, found {call_sites:?}"
        );
        assert_eq!(
            call_sites[0].0, "routes.rs",
            "the only call must be in the HTTP layer"
        );

        // And that place must be `deliver`, so moving the call to another handler fails.
        //
        // This used to require `approve`. ADR-0010 added auto-send as a second authorised
        // caller, and the call moved into `deliver` — one function both go through, so the
        // guard sequence cannot be duplicated and drift. The assertion changed with it
        // rather than being deleted: the invariant is still "one call site", and it is
        // still checked, but it now names the function that actually holds it.
        let routes = &routes_without_tests;
        let call_line = call_sites[0].1;
        let before = routes
            .lines()
            .take(call_line)
            .collect::<Vec<_>>()
            .join("\n");
        let enclosing = before.rsplit("async fn ").next().unwrap_or("");
        assert!(
            enclosing.starts_with("deliver"),
            "the only Slack write must be inside `deliver`, found it inside `{}`",
            enclosing.split('(').next().unwrap_or("?")
        );
    }

    /// ADR-0010: exactly two things may reach `deliver` — a human clicking Approve, and
    /// auto-send from the scheduler. A third caller would be a path to Slack nobody chose.
    #[test]
    fn only_approve_and_auto_send_reach_deliver() {
        let sources: [(&str, &str); 3] = [
            ("routes.rs", include_str!("routes.rs")),
            ("scheduler.rs", include_str!("scheduler.rs")),
            ("main.rs", include_str!("main.rs")),
        ];

        let mut callers: Vec<(&str, String)> = Vec::new();
        for (name, source) in sources {
            let source_lines: Vec<&str> = production(source).lines().collect();
            for (index, raw) in source_lines.iter().enumerate() {
                let line = raw;
                let code = line.split("//").next().unwrap_or("").trim();
                if code.is_empty() || code.starts_with("//") {
                    continue;
                }
                // The definition itself. A doc comment may also mention `deliver`, but a `///` line
                // strips to empty above.
                if code.contains("fn deliver(") {
                    continue;
                }
                // `deliver(` also matches `crate::routes::deliver(`, which is the form
                // the scheduler uses — matching on the bare name would miss it, which is
                // exactly how a test could pass while the thing it guards was unguarded.
                //
                // The `is_deliver_call` guard keeps this test's own source out: the string
                // literals here contain "deliver(", and it reads its own file.
                // `deliver(` also matches `crate::routes::deliver(`, which is the form
                // the scheduler uses, so matching on the bare name is deliberate. The
                // test module is already excluded by `production`, so this cannot match
                // itself.
                if code.contains("deliver(") {
                    // Name the enclosing fn so the failure says who posted.
                    let before = source_lines[..=index].join("\n");
                    let enclosing = before
                        .rsplit("async fn ")
                        .next()
                        .unwrap_or("")
                        .split('(')
                        .next()
                        .unwrap_or("?")
                        .to_string();
                    callers.push((name, enclosing));
                }
            }
        }

        assert_eq!(
            callers.len(),
            2,
            "exactly two callers may reach Slack — `approve` and `auto_send` — found {callers:?}"
        );
        let names: Vec<&str> = callers.iter().map(|(_, f)| f.as_str()).collect();
        assert!(
            names.contains(&"approve") && names.contains(&"auto_send"),
            "the two callers must be `approve` and `auto_send`, found {names:?}"
        );
    }

    #[test]
    fn a_fire_outcome_code_is_shown_for_every_terminal_state() {
        // The spine's code column must be able to label any row it is given.
        for outcome in [
            FireOutcome::Drafted,
            FireOutcome::Partial,
            FireOutcome::Failed,
            FireOutcome::Missed,
            FireOutcome::Skipped,
        ] {
            assert!(!outcome.code().is_empty());
        }
    }
}
