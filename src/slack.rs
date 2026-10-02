//! Slack Web API client.
//!
//! Authenticates with a user session token and its `d` cookie (ADR-0002). Both halves are
//! required: dropping either yields `invalid_auth`, so they are taken together and never
//! separately.
//!
//! No rate-limit headers are published for this token type, so pacing is self-imposed at
//! one request per second rather than read from `Retry-After`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::secrets::{Secret, SecretError};

const BASE: &str = "https://slack.com/api";
/// Tier 3 is 50+/min, but nothing tells us when a window is safe, so pace conservatively.
const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1000);
/// A page budget, so a chatty channel cannot stall a Fire indefinitely.
const MAX_PAGES: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum SlackError {
    #[error("secret store: {0}")]
    Secrets(#[from] SecretError),
    #[error("slack returned {code}: {message}")]
    Api { code: String, message: String },
    /// The cookie died with the web session. Distinguished from other API errors because
    /// it is the one failure the user can fix, and the message they will actually read.
    #[error("slack rejected the session ({code}); the cookie expired — log in to Slack in a browser and paste the token and `d` cookie again")]
    SessionExpired { code: String },
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("slack response was not valid json: {0}")]
    Decode(String),
}

/// Slack's envelope. `ok: false` is returned as a 200, so the body decides.
#[derive(Debug, Deserialize)]
#[serde(bound = "T: serde::de::DeserializeOwned")]
struct ApiResponse<T> {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    needed: Option<String>,
    #[serde(default)]
    provided: Option<String>,
    #[serde(default)]
    response_metadata: Option<ResponseMetadata>,
    #[serde(flatten)]
    payload: T,
}

#[derive(Debug, Default, Deserialize)]
struct ResponseMetadata {
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct HistoryPayload {
    #[serde(default)]
    messages: Vec<SlackMessage>,
}

#[derive(Debug, Default, Deserialize)]
struct ListPayload {
    #[serde(default)]
    channels: Vec<SlackChannel>,
}

#[derive(Debug, Default, Deserialize)]
struct RepliesPayload {
    #[serde(default)]
    messages: Vec<SlackMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackChannel {
    #[allow(dead_code)]
    pub id: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackMessage {
    /// Slack's string timestamp, e.g. `"1759392000.000400"`.
    #[serde(rename = "ts")]
    pub ts: String,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default = "default_subtype")]
    pub subtype: Option<String>,
    /// Read by the live source to decide whether a thread is worth reading.
    #[serde(default)]
    #[allow(dead_code)]
    pub reply_count: Option<u32>,
}

fn default_subtype() -> Option<String> {
    Some("message".to_string())
}

impl SlackMessage {
    pub fn as_time(&self) -> Option<DateTime<Utc>> {
        let seconds = self.ts.split('.').next()?;
        let secs: i64 = seconds.parse().ok()?;
        DateTime::from_timestamp(secs, 0)
    }

    /// Whether this is the user's own post rather than someone else's or a join notice.
    pub fn is_from(&self, user_id: &str) -> bool {
        // A bot_message subtype means it came from an app, not the person.
        self.subtype.as_deref() == Some("message") && self.user.as_deref() == Some(user_id)
    }
}

/// A client for one Slack session.
///
/// Cheap to clone: `reqwest::Client` pools its own connections, and a cloned client
/// shares that pool rather than opening new ones.
#[derive(Clone)]
pub struct SlackClient {
    http: reqwest::Client,
    token: Secret,
    cookie: Secret,
    /// The user's own `U…` id. Activity is filtered to their own posts.
    user_id: String,
    last_request: Arc<tokio::sync::Mutex<Option<std::time::Instant>>>,
    base: String,
}

impl SlackClient {
    pub fn new(token: Secret, cookie: Secret, user_id: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            token,
            cookie,
            user_id: user_id.into(),
            last_request: Arc::new(tokio::sync::Mutex::new(None)),
            base: BASE.to_string(),
        }
    }

    /// Point at a test server. Never used in production.
    #[allow(dead_code)]
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_string();
        self
    }

    #[allow(dead_code)]
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// Self-imposed pacing. Slack publishes no headers for this token, so there is
    /// nothing to read — this is the whole rate-limit strategy.
    async fn pace(&self) {
        let wait_until = {
            let last = self.last_request.lock().await;
            match *last {
                Some(prev) => prev + MIN_INTERVAL,
                None => std::time::Instant::now(),
            }
        };
        let now = std::time::Instant::now();
        if wait_until > now {
            tokio::time::sleep(wait_until - now).await;
        }
        *self.last_request.lock().await = Some(std::time::Instant::now());
    }

    /// One authenticated call. Both credentials on every request, always.
    ///
    /// Returns the payload and the paging cursor together: the cursor lives in the
    /// envelope, so a caller that only received the payload could not page without a
    /// second round trip for metadata it already had.
    async fn call<T: serde::de::DeserializeOwned + Default>(
        &self,
        method: &str,
        query: &[(&str, String)],
    ) -> Result<(T, Option<String>), SlackError> {
        self.pace().await;

        let mut request = self
            .http
            .get(format!("{}/{}", self.base, method))
            // The two halves of one session. ADR-0002.
            .bearer_auth(self.token.expose())
            // Passed exactly as the browser sent it, percent-escapes intact. Decoding
            // it is the most likely way to break an otherwise-correct setup, and it
            // presents as a wrong token rather than a mangled cookie.
            .header(
                reqwest::header::COOKIE,
                format!("d={}", self.cookie.expose()),
            )
            .header(
                "Content-Type",
                "application/x-www-form-urlencoded; charset=utf-8",
            );

        for (key, value) in query {
            request = request.query(&[(key, value.as_str())]);
        }

        let text = request.send().await?.text().await?;
        let response: ApiResponse<T> =
            serde_json::from_str(&text).map_err(|e| SlackError::Decode(e.to_string()))?;

        if !response.ok {
            let code = response.error.unwrap_or_else(|| "unknown".into());
            let message = match (response.needed.as_deref(), response.provided.as_deref()) {
                (Some(needed), Some(provided)) => {
                    format!("needed {needed}, provided {provided}")
                }
                _ => code.clone(),
            };

            return Err(
                if code == "invalid_auth" || code == "not_authed" || code == "token_revoked" {
                    SlackError::SessionExpired { code }
                } else {
                    SlackError::Api { code, message }
                },
            );
        }

        let cursor = response
            .response_metadata
            .and_then(|m| m.cursor)
            .filter(|c| !c.is_empty() && c != "undefined");

        Ok((response.payload, cursor))
    }

    /// `auth.test` — cheap, exercises both credentials together.
    ///
    /// The one call worth keeping at setup: its failure message is what a user reads at
    /// 6pm when the cookie has expired (ADR-0008).
    pub async fn verify(&self) -> Result<String, SlackError> {
        #[derive(Deserialize, Default)]
        struct AuthTest {
            #[serde(default)]
            user_id: String,
        }

        let (payload, _): (AuthTest, Option<String>) = self.call("auth.test", &[]).await?;
        Ok(payload.user_id)
    }

    /// The user identity, used to discover which identity this install is about.
    #[allow(dead_code)]
    pub async fn whoami(&self) -> Result<String, SlackError> {
        self.verify().await
    }

    /// Messages in a channel between two instants, filtered to the user's own posts.
    ///
    /// Pages until Slack stops offering a cursor, capped at `MAX_PAGES` so one very
    /// chatty channel cannot stall a Fire indefinitely.
    /// Messages in a channel, with whether paging stopped at the page cap.
    ///
    /// The caller needs the second value: a cap that silently truncates makes the
    /// Evidence understate the day in the reassuring direction, which is the one
    /// direction ADR-0005 exists to prevent.
    pub async fn history_with_cap(
        &self,
        channel: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<(Vec<SlackMessage>, bool), SlackError> {
        let mut collected: Vec<SlackMessage> = Vec::new();
        let mut cursor: Option<String> = None;
        let user = self.user_id.clone();

        for _ in 0..MAX_PAGES {
            let mut query: Vec<(&str, String)> = vec![
                ("channel", channel.to_string()),
                ("oldest", from.timestamp().to_string()),
                ("latest", to.timestamp().to_string()),
                ("limit", "200".to_string()),
            ];
            if let Some(c) = &cursor {
                query.push(("cursor", c.clone()));
            }

            let (payload, next): (HistoryPayload, Option<String>) =
                self.call("conversations.history", &query).await?;

            collected.extend(
                payload
                    .messages
                    .into_iter()
                    .filter(|m| m.is_from(&user))
                    .map(|mut m| {
                        // `user` was only a discriminator for the filter above. Drop it so
                        // the renderer never has to reason about whose message this is.
                        m.user = None;
                        m
                    }),
            );

            match next {
                Some(c) => cursor = Some(c),
                // No cursor means Slack has nothing more. Return the flag so the caller
                // can distinguish "read everything" from "hit the cap".
                None => return Ok((collected, false)),
            }
        }

        // A cursor is still being offered at the cap, so more messages exist than we read.
        Ok((collected, cursor.is_some()))
    }

    /// Channels the user is a member of, for activity-based discovery.
    #[allow(dead_code)]
    pub async fn channels(&self) -> Result<Vec<SlackChannel>, SlackError> {
        let mut out: Vec<SlackChannel> = Vec::new();
        let mut cursor: Option<String> = None;

        for _ in 0..MAX_PAGES {
            let mut query: Vec<(&str, String)> = vec![("limit", "200".to_string())];
            if let Some(c) = &cursor {
                query.push(("cursor", c.clone()));
            }

            let (payload, next): (ListPayload, Option<String>) =
                self.call("conversations.list", &query).await?;
            out.extend(payload.channels);

            match next {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }

        Ok(out)
    }

    /// Replies in a thread, filtered to the user's own posts.
    ///
    /// A reply is real work that a strict self-filter would hide, so a Fire that saw a
    /// thread worth reading also reads the thread.
    #[allow(dead_code)]
    pub async fn replies(&self, channel: &str, ts: &str) -> Result<Vec<SlackMessage>, SlackError> {
        let (payload, _): (RepliesPayload, Option<String>) = self
            .call(
                "conversations.replies",
                &[
                    ("channel", channel.to_string()),
                    ("ts", ts.to_string()),
                    ("limit", "200".to_string()),
                ],
            )
            .await?;

        let user = self.user_id.clone();
        Ok(payload
            .messages
            .into_iter()
            .filter(|m| m.is_from(&user))
            .collect())
    }

    /// `chat.postMessage` — the only mutating call in the entire tool.
    ///
    /// Called from exactly one place: the Approve handler. ADR-0001.
    pub async fn post(&self, channel: &str, text: &str) -> Result<String, SlackError> {
        #[derive(Deserialize, Default)]
        struct PostPayload {
            #[serde(default)]
            ts: String,
        }

        self.pace().await;

        let body = serde_json::json!({ "channel": channel, "text": text });
        let text_body = self
            .http
            .post(format!("{}/chat.postMessage", self.base))
            .bearer_auth(self.token.expose())
            .header(
                reqwest::header::COOKIE,
                format!("d={}", self.cookie.expose()),
            )
            .header("Content-Type", "application/json; charset=utf-8")
            .body(serde_json::to_string(&body).unwrap_or_default())
            .send()
            .await?
            .text()
            .await?;

        let response: ApiResponse<PostPayload> =
            serde_json::from_str(&text_body).map_err(|e| SlackError::Decode(e.to_string()))?;

        if !response.ok {
            let code = response.error.unwrap_or_else(|| "unknown".into());
            // Same mapping as every read: an expired session must say so with
            // instructions, not "slack returned invalid_auth". This is the message a
            // user reads at 6pm when the cookie has died (ADR-0002, ADR-0008).
            return Err(
                if matches!(
                    code.as_str(),
                    "invalid_auth" | "not_authed" | "token_revoked"
                ) {
                    SlackError::SessionExpired { code }
                } else {
                    SlackError::Api {
                        message: code.clone(),
                        code,
                    }
                },
            );
        }

        Ok(response.payload.ts)
    }
}

// ── helpers ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> SlackClient {
        SlackClient::new(
            Secret::new("xoxc-test"),
            Secret::new("xoxd-test"),
            "U0A9WPY4S1F",
        )
        .with_base_url(server.uri())
    }

    #[tokio::test]
    async fn verify_returns_the_user_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth.test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true, "user_id": "U0A9WPY4S1F", "team": "Codeacious Tech"
            })))
            .mount(&server)
            .await;

        assert_eq!(client(&server).verify().await.unwrap(), "U0A9WPY4S1F");
    }

    #[tokio::test]
    async fn an_expired_cookie_is_named_as_such() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth.test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false, "error": "invalid_auth"
            })))
            .mount(&server)
            .await;

        let err = client(&server).verify().await.unwrap_err();
        // The message a user reads at 6pm has to tell them what to actually do.
        assert!(matches!(err, SlackError::SessionExpired { .. }));
        assert!(err.to_string().contains("log in to Slack"));
    }

    #[tokio::test]
    async fn not_authed_is_treated_as_an_expired_session() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth.test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false, "error": "not_authed"
            })))
            .mount(&server)
            .await;

        assert!(matches!(
            client(&server).verify().await,
            Err(SlackError::SessionExpired { .. })
        ));
    }

    #[tokio::test]
    async fn other_api_errors_are_not_mislabelled_as_session_expiry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/auth.test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false, "error": "ratelimited"
            })))
            .mount(&server)
            .await;

        // Misreporting a rate limit as "log in again" would send the user chasing the
        // wrong problem.
        assert!(matches!(
            client(&server).verify().await,
            Err(SlackError::Api { .. })
        ));
    }

    #[tokio::test]
    async fn history_keeps_only_the_users_own_messages() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/conversations.history"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "messages": [
                    {"ts": "1759392000.0001", "user": "U0A9WPY4S1F", "text": "mine"},
                    {"ts": "1759392100.0001", "user": "UOTHER", "text": "theirs"},
                    {"ts": "1759392200.0001", "text": "a join notice", "subtype": "channel_join"},
                    {"ts": "1759392300.0001", "user": "U0A9WPY4S1F", "text": "mine too"}
                ]
            })))
            .mount(&server)
            .await;

        let messages = client(&server)
            .history("C1", Utc::now() - chrono::Duration::hours(1), Utc::now())
            .await
            .unwrap();

        let texts: Vec<&str> = messages.iter().filter_map(|m| m.text.as_deref()).collect();
        assert_eq!(texts, vec!["mine", "mine too"]);
    }

    #[tokio::test]
    async fn post_returns_the_message_timestamp() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat.postMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true, "ts": "1759392400.000300", "channel": "C1"
            })))
            .mount(&server)
            .await;

        let ts = client(&server).post("C1", "Day Task:").await.unwrap();
        assert_eq!(ts, "1759392400.000300");
    }

    #[tokio::test]
    async fn message_timestamps_parse_from_slacks_format() {
        let message = SlackMessage {
            ts: "1759392000.000400".into(),
            user: None,
            text: None,
            subtype: None,
            reply_count: None,
        };
        let parsed = message.as_time().unwrap();
        assert_eq!(parsed.timestamp(), 1759392000);
    }

    #[tokio::test]
    async fn a_malformed_timestamp_yields_none_rather_than_panicking() {
        let message = SlackMessage {
            ts: "not-a-timestamp".into(),
            user: None,
            text: None,
            subtype: None,
            reply_count: None,
        };
        assert!(message.as_time().is_none());
    }
}
