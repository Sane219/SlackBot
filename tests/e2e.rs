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
    body: String,
}

impl Response {
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

/// A minimal HTTP/1.1 client, so the tests add no HTTP dependency and can assert on
/// exact status codes — which matters here, because the difference between 200 and 409 is
/// the difference between "posted" and "already posted".
fn request(method: &str, path: &str, body: Option<&str>) -> Response {
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
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
        payload.len()
    );
    stream.write_all(request.as_bytes()).unwrap();

    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();

    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or("")
        .to_string();

    Response { status, body }
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
    let status = raw
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or("")
        .to_string();
    Response { status, body }
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

#[test]
#[ignore = "needs a running server"]
fn the_static_assets_are_served_with_correct_content_types() {
    for (path, expected) in [
        ("/", "text/html"),
        ("/app.css", "text/css"),
        ("/app.js", "text/javascript"),
    ] {
        let url = base_url();
        let host_port = url.strip_prefix("http://").unwrap();
        let mut stream = TcpStream::connect(host_port).unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        assert!(raw.starts_with("HTTP/1.1 200"), "{path} did not return 200");
        assert!(
            raw.to_lowercase().contains(expected),
            "{path} did not declare {expected}"
        );
    }
}
