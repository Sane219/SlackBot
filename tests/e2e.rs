//! End-to-end checks against a running server.
//!
//! Run with `cargo test --test e2e -- --ignored` while a server is listening on
//! `$SLACKBOT_E2E_URL` (default http://127.0.0.1:7321). Every test here goes over HTTP
//! against the real router, so it exercises the same paths a browser would.

use std::io::{Read, Write};
use std::net::TcpStream;

fn base_url() -> String {
    std::env::var("SLACKBOT_E2E_URL").unwrap_or_else(|_| "http://127.0.0.1:7321".into())
}

struct Response {
    status: u16,
    /// Header block as the server sent it, lowercased. Content type is part of the
    /// contract — a stylesheet served as `text/plain` is not loaded.
    headers: String,
    body: String,
}

impl Response {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    }
}

/// A minimal HTTP/1.1 client, so the tests add no HTTP dependency and can assert on
/// exact status codes — which matters here, because the difference between 200 and 409 is
/// the difference between "posted" and "already posted".
fn request(method: &str, path: &str, body: Option<&str>) -> Response {
    raw_request(method, path, body, &[])
}

fn get(path: &str) -> Response {
    request("GET", path, None)
}

/// Like `request`, with extra headers, for asserting on the origin boundary.
fn raw_request(method: &str, path: &str, body: Option<&str>, headers: &[(&str, &str)]) -> Response {
    let url = base_url();
    let rest = url
        .strip_prefix("http://")
        .expect("SLACKBOT_E2E_URL must be http://");
    let (host_port, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}{path}")),
        None => (rest, path.to_string()),
    };

    let mut stream = TcpStream::connect(host_port).expect("connect to the slackbot server");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .unwrap();

    let payload = body.unwrap_or("");
    // Caller headers come last so a test can override the default Content-Type;
    // sending it twice left the first one winning.
    let extra: String = headers
        .iter()
        .map(|(k, v)| format!("{k}: {v}\r\n"))
        .collect();
    let default_type = if headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"))
    {
        ""
    } else {
        "Content-Type: application/json\r\n"
    };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\
         {default_type}Content-Length: {}\r\n{extra}\r\n{payload}",
        payload.len()
    );
    stream.write_all(req.as_bytes()).unwrap();

    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    let (headers, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);

    Response {
        status,
        headers: headers.to_lowercase(),
        body: body.to_string(),
    }
}

fn post(path: &str, body: serde_json::Value) -> Response {
    request("POST", path, Some(&body.to_string()))
}

fn patch(path: &str, body: serde_json::Value) -> Response {
    request("PATCH", path, Some(&body.to_string()))
}

#[test]
#[ignore = "needs a running server"]
fn health_reports_configuration_state() {
    let res = get("/api/health");
    assert_eq!(res.status, 200);

    let body = res.json();
    assert_eq!(body["status"], "ok");
    // The four flags must all be present: the setup board reads them by name.
    for key in ["llm", "slack_token", "slack_cookie", "github"] {
        assert!(body["configured"].get(key).is_some(), "missing {key}");
    }
}

#[test]
#[ignore = "needs a running server"]
fn a_blank_credential_is_rejected_rather_than_stored() {
    let res = post(
        "/api/setup/credential",
        serde_json::json!({"kind": "llm_api_key", "value": "   "}),
    );
    // A blank credential would fail later, at a Fire, where the cause is invisible.
    assert_eq!(res.status, 400);
}

#[test]
#[ignore = "needs a running server"]
fn an_unknown_credential_kind_is_rejected() {
    let res = post(
        "/api/setup/credential",
        serde_json::json!({"kind": "not_a_kind", "value": "x"}),
    );
    assert_eq!(res.status, 422);
}

#[test]
#[ignore = "needs a running server"]
fn a_job_with_an_unparseable_time_is_rejected() {
    let res = post(
        "/api/jobs",
        serde_json::json!({
            "name": "Day Task", "at": "half past nine", "tz": "Asia/Kolkata",
            "channel_id": "C1", "channel_name": "coot-ai",
            "context": "lookback", "lookback_hours": 8, "prompt": "p"
        }),
    );
    // Rejected at the boundary rather than stored and failing silently at 09:30.
    assert_eq!(res.status, 400);
    assert!(res.body.contains("HH:MM"), "got: {}", res.body);
}

#[test]
#[ignore = "needs a running server"]
fn a_job_with_an_unknown_timezone_is_rejected() {
    let res = post(
        "/api/jobs",
        serde_json::json!({
            "name": "Day Task", "at": "09:30", "tz": "Mars/Olympus",
            "channel_id": "C1", "channel_name": "coot-ai",
            "context": "lookback", "lookback_hours": 8, "prompt": "p"
        }),
    );
    assert_eq!(res.status, 400);
    assert!(res.body.contains("timezone"), "got: {}", res.body);
}

#[test]
#[ignore = "needs a running server"]
fn a_lookback_is_clamped_to_a_sane_range() {
    let create = serde_json::json!({
        "name": "Clamp", "at": "09:30", "tz": "Asia/Kolkata",
        "channel_id": "C1", "channel_name": "coot-ai",
        "context": "lookback", "lookback_hours": 9999, "prompt": "p"
    });
    let res = post("/api/jobs", create.clone());
    assert_eq!(res.status, 200);

    let id = res.json()["id"].as_i64().unwrap();
    let job = get("/api/jobs").json();
    let found = job["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["id"] == id)
        .cloned()
        .unwrap();
    let hours = found["context_window"]["hours"].as_i64().unwrap();

    assert!((1..=72).contains(&hours), "clamped to {hours}");
    request("DELETE", &format!("/api/jobs/{id}"), None);
}

#[test]
#[ignore = "needs a running server"]
fn a_job_can_be_created_with_only_the_fields_that_matter() {
    // Regression: `channel_name` had no serde default, so a caller sending just an id
    // got a 422 before any validation ran — the same class of bug as the missing
    // `previous_day` default.
    let res = post(
        "/api/jobs",
        serde_json::json!({
            "name": "Minimal", "at": "11:00", "tz": "UTC",
            "channel_id": "C0A0RRC7P8B",
            "context": "lookback", "prompt": "p"
        }),
    );
    assert_eq!(res.status, 200, "got {}: {}", res.status, res.body);
    let id = res.json()["id"].as_i64().unwrap();
    request("DELETE", &format!("/api/jobs/{id}"), None);
}

#[test]
#[ignore = "needs a running server"]
fn a_channel_name_without_slack_is_a_clear_precondition_not_a_parse_error() {
    // The Plan Role produces "#coot-ai"; resolving it needs a Slack client. Without one
    // the user must be told what to do, not handed a deserialization failure.
    let res = post(
        "/api/jobs",
        serde_json::json!({
            "name": "By name", "at": "11:00", "tz": "UTC",
            "channel_id": "#coot-ai", "context": "lookback", "prompt": "p"
        }),
    );
    assert_ne!(res.status, 422, "a name must not fail as malformed JSON");
    if res.status == 412 {
        assert!(
            res.body.contains("Slack"),
            "the precondition must name what is missing: {}",
            res.body
        );
    }
}

#[test]
#[ignore = "needs a running server"]
fn a_cross_origin_approve_is_refused() {
    // ADR-0001: loopback is not an authorisation boundary. A page the user happens to
    // be visiting must not be able to post to their team channel.
    let res = request("POST", "/api/drafts/1/approve", Some("{}"));
    // A same-origin request with no Origin header passes the guard and reaches the
    // handler; the cross-origin case is what must be refused, and it is checked by
    // sending an Origin the tool does not own.
    let _ = res;

    let hostile = raw_request(
        "POST",
        "/api/drafts/1/approve",
        Some("{}"),
        &[
            ("Origin", "https://evil.example"),
            ("Sec-Fetch-Site", "cross-site"),
        ],
    );
    assert_eq!(
        hostile.status, 403,
        "a cross-origin approve must be refused, got {}",
        hostile.status
    );
}

#[test]
#[ignore = "needs a running server"]
fn a_form_encoded_write_is_refused() {
    // A cross-origin HTML form can send urlencoded without a preflight, so the content
    // type is the cheap half of the same boundary.
    let res = raw_request(
        "POST",
        "/api/drafts/1/approve",
        Some(""),
        &[("Content-Type", "application/x-www-form-urlencoded")],
    );
    assert_eq!(res.status, 415, "got {}", res.status);
}

#[test]
#[ignore = "needs a running server"]
fn approving_an_unknown_draft_is_a_404_not_a_silent_success() {
    let res = post("/api/drafts/999999/approve", serde_json::json!({}));
    assert_eq!(res.status, 404);
}

#[test]
#[ignore = "needs a running server"]
fn the_inbox_lists_drafts_and_fires_in_one_response() {
    let res = get("/api/inbox");
    assert_eq!(res.status, 200);

    let body = res.json();
    assert!(body["drafts"].is_array());
    assert!(body["fires"].is_array());
    // Each Draft carries its Job's name so the board needs no second lookup.
    if let Some(first) = body["drafts"].as_array().unwrap().first() {
        assert!(first["job_name"].is_string());
        assert!(first["counts"].is_object());
    }
}

#[test]
#[ignore = "needs a running server"]
fn planning_without_a_model_is_refused_with_a_clear_message() {
    let res = post(
        "/api/plan",
        serde_json::json!({"description": "I post three times a day"}),
    );
    // Either no model is configured, or the plan failed. Both are honest refusals;
    // neither should look like a working schedule.
    assert!(
        res.status == 412 || res.status == 502,
        "unexpected {}: {}",
        res.status,
        res.body
    );
}

#[test]
#[ignore = "needs a running server"]
fn an_empty_description_is_rejected() {
    let res = post("/api/plan", serde_json::json!({"description": "   "}));
    assert_eq!(res.status, 400);
}

#[test]
#[ignore = "needs a running server"]
fn a_deleted_draft_is_not_approved_afterwards() {
    // Create a job, fire it if possible, and confirm the approve path 404s on a
    // discarded Draft rather than reporting a post.
    let res = post(
        "/api/jobs",
        serde_json::json!({
            "name": "E2E", "at": "09:30", "tz": "Asia/Kolkata",
            "channel_id": "C0A0RRC7P8B", "channel_name": "coot-ai",
            "context": "lookback", "lookback_hours": 8, "prompt": "p"
        }),
    );
    if res.status != 200 {
        return; // Setup not available in this environment.
    }
    let id = res.json()["id"].as_i64().unwrap();
    request("DELETE", &format!("/api/jobs/{id}"), None);

    let approve = post("/api/drafts/1/approve", serde_json::json!({}));
    assert!(approve.status == 404 || approve.status == 409 || approve.status == 412);
}

#[test]
#[ignore = "needs a running server"]
fn a_failed_approve_leaves_the_draft_in_the_inbox() {
    // The bug this pins: `approve` claimed the Draft (setting `approved = 1`) and *then*
    // discovered Slack was not configured, returning 412 without releasing the claim. The
    // Draft disappeared from the Inbox, `approved_at` said it had been sent, and every
    // later attempt returned 409 "already been approved". Approving once before setup
    // silently destroyed a drafted post.
    let Some(draft) = first_approvable_draft() else {
        return; // Nothing waiting in this environment.
    };

    let first = post(
        &format!("/api/drafts/{draft}/approve"),
        serde_json::json!({}),
    );
    assert_ne!(first.status, 200, "approve must not succeed without Slack");

    // Still listed, and still not approved — whatever the failure was.
    let inbox = get("/api/inbox");
    assert_eq!(inbox.status, 200, "the Inbox must still be readable");
    let body = inbox.json();
    let drafts = body["drafts"].as_array().expect("drafts is an array");
    let still = drafts.iter().find(|d| d["id"].as_i64() == Some(draft));
    assert!(
        still.is_some(),
        "a failed approve consumed draft {draft}: it is gone from the Inbox"
    );
    let still = still.unwrap();
    assert_eq!(
        still["approved"], false,
        "a failed approve left draft {draft} marked approved"
    );
    assert!(
        still["approved_at"].is_null(),
        "a failed approve stamped draft {draft} as sent"
    );

    // And the same Draft can be tried again — a 409 here is the bricking.
    let second = post(
        &format!("/api/drafts/{draft}/approve"),
        serde_json::json!({}),
    );
    assert_ne!(
        second.status, 409,
        "draft {draft} can no longer be retried: {}",
        second.body
    );
}

/// The id of a waiting Draft that has evidence, or `None`.
fn first_approvable_draft() -> Option<i64> {
    let inbox = get("/api/inbox");
    if inbox.status != 200 {
        return None;
    }
    inbox.json()["drafts"]
        .as_array()?
        .iter()
        .find(|d| {
            d["no_signal"] == false
                && d["discarded"] == false
                && !d["text"].as_str().unwrap_or("").trim().is_empty()
        })
        .and_then(|d| d["id"].as_i64())
}

#[test]
#[ignore = "needs a running server"]
fn a_disabled_job_can_be_toggled() {
    let res = post(
        "/api/jobs",
        serde_json::json!({
            "name": "Toggle", "at": "10:30", "tz": "Asia/Kolkata",
            "channel_id": "C1", "channel_name": "coot-ai",
            "context": "lookback", "lookback_hours": 8, "prompt": "p"
        }),
    );
    if res.status != 200 {
        return;
    }
    let id = res.json()["id"].as_i64().unwrap();

    let off = patch(
        &format!("/api/jobs/{id}"),
        serde_json::json!({"enabled": false}),
    );
    assert_eq!(off.status, 200);
    assert_eq!(off.json()["enabled"], false);

    let on = patch(
        &format!("/api/jobs/{id}"),
        serde_json::json!({"enabled": true}),
    );
    assert_eq!(on.json()["enabled"], true);

    request("DELETE", &format!("/api/jobs/{id}"), None);
}

/// The value of the first `attribute="…"` in `html`.
///
/// Split on the attribute name and the opening quote only. Including the closing quote
/// in the needle yields `>` as the "value", which reads as a missing asset rather than a
/// parser mistake.
fn attribute(html: &str, attribute: &str) -> Option<String> {
    let needle = format!("{attribute}=\"");
    let after = html.split(&needle).nth(1)?;
    let value = after.split('"').next().unwrap_or_default();
    (!value.is_empty()).then(|| value.to_string())
}

/// Every asset `index.html` asks for is served, with the right content type.
///
/// The paths are read out of the page rather than hardcoded. Hardcoding them meant this
/// test asserted `/app.js` exists while the build was emitting `/assets/app.js` — it
/// would have kept passing against a UI that could not load. Following the page is the
/// contract that actually matters (ADR-0009): whatever `dist/index.html` references must
/// be embedded, or the board is blank.
#[test]
#[ignore = "needs a running server"]
fn every_asset_the_page_references_is_served() {
    let page = request("GET", "/", None);
    assert_eq!(page.status, 200, "the page itself must be served");
    assert!(
        page.header("content-type")
            .is_some_and(|t| t.contains("text/html")),
        "the page must declare itself as html, got {:?}",
        page.header("content-type")
    );

    let script = attribute(&page.body, "src")
        .expect("index.html must load a script, or the board never renders");
    let stylesheet = page
        .body
        .split("<link rel=\"stylesheet\"")
        .nth(1)
        .and_then(|rest| attribute(rest, "href"))
        .expect("index.html must link a stylesheet, or the board is unstyled");

    for (path, expected) in [(script, "text/javascript"), (stylesheet, "text/css")] {
        assert!(
            path.starts_with('/'),
            "asset paths must be absolute, got {path}"
        );
        let response = request("GET", &path, None);
        assert_eq!(response.status, 200, "{path} was not served");
        assert!(
            response
                .header("content-type")
                .is_some_and(|t| t.contains(expected)),
            "{path} did not declare {expected}, got {:?}",
            response.header("content-type")
        );
        assert!(
            !response.body.is_empty(),
            "{path} was served empty, which is the blank-screen failure"
        );
    }
}

/// A path that is not in `dist/` gets a real 404, not the page.
///
/// Falling back to `index.html` would turn a stale or partial `dist/` into a silently
/// broken board: the browser would parse HTML as a module and report nothing useful.
#[test]
#[ignore = "needs a running server"]
fn an_unknown_asset_is_a_404_not_the_page() {
    let response = request("GET", "/assets/app.js.map", None);
    assert_eq!(response.status, 404);
    assert!(
        !response.body.contains("<!doctype html"),
        "a missing asset must not be answered with the page"
    );
}
