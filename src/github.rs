//! GitHub client, reading the user's own comments rather than searching for them.
//!
//! Search is deliberately not used. The `commenter:` qualifier matches issue comments
//! only, so a reviewer's review comments — the bulk of a PM's or tester's work — do not
//! appear in its results at all. Per-repo `/issues/comments` and `/pulls/comments` with
//! `since` are both correct and cheaper.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::secrets::{Secret, SecretError};

const BASE: &str = "https://api.github.com";

#[derive(Debug, thiserror::Error)]
pub enum GithubError {
    #[error("secret store: {0}")]
    Secrets(#[from] SecretError),
    #[error("github returned {status}: {message}")]
    Api { status: u16, message: String },
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("github response was not valid json: {0}")]
    Decode(String),
}

/// One thing the user did, from either comment endpoint.
///
/// Constructed internally rather than parsed, so it carries no `Deserialize`.
#[derive(Debug, Clone)]
pub struct GithubActivity {
    /// `owner/repo`.
    pub repo: String,
    pub number: u64,
    /// Where this came from. The two endpoints overlap on PR conversation comments, so
    /// the source matters for not counting the same comment twice.
    pub kind: ActivityKind,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    /// A comment on an issue or on a PR's conversation tab.
    IssueComment,
    /// A review-thread comment: the work a reviewer actually does.
    ReviewComment,
}

impl ActivityKind {
    pub fn code(&self) -> &'static str {
        match self {
            ActivityKind::IssueComment => "MSG",
            ActivityKind::ReviewComment => "PR",
        }
    }
}

#[derive(Debug, Deserialize)]
struct IssueCommentWire {
    body: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    /// Kept so a Draft can cite where a comment came from. The renderer does not
    /// emit links yet, so this is currently unread.
    #[allow(dead_code)]
    html_url: Option<String>,
    #[serde(default)]
    issue: Option<IssueRef>,
}

#[derive(Debug, Deserialize)]
struct IssueRef {
    number: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ReviewCommentWire {
    body: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    /// Kept so a Draft can cite where a comment came from. The renderer does not
    /// emit links yet, so this is currently unread.
    #[allow(dead_code)]
    html_url: Option<String>,
    #[serde(default)]
    pull_request: Option<PullRef>,
}

#[derive(Debug, Deserialize)]
struct PullRef {
    number: u64,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Clone)]
pub struct GithubClient {
    http: reqwest::Client,
    token: Secret,
    /// The login whose comments count as this user's work.
    login: String,
    base: String,
}

impl GithubClient {
    pub fn new(token: Secret, login: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            token,
            login: login.into(),
            base: BASE.to_string(),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_string();
        self
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, GithubError> {
        let text = self
            .http
            .get(format!("{}/{}", self.base, path.trim_start_matches('/')))
            .bearer_auth(self.token.expose())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?
            .text()
            .await?;

        // A GitHub error body is JSON too, so a status check has to come first or a 404
        // would try to deserialize as the success type and report a decode failure
        // instead of the real problem.
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(message) = value.get("message").and_then(|m| m.as_str()) {
                return Err(GithubError::Api {
                    status: value.get("status").and_then(|s| s.as_u64()).unwrap_or(0) as u16,
                    message: message.to_string(),
                });
            }
        }

        serde_json::from_str(&text).map_err(|e| GithubError::Decode(e.to_string()))
    }

    /// The authenticated login. Also the setup-time verification (ADR-0008).
    pub async fn whoami(&self) -> Result<String, GithubError> {
        #[derive(Deserialize)]
        struct User {
            login: String,
        }
        let user: User = self.get("user").await?;
        Ok(user.login)
    }

    /// The user's own comments in one repository within a window.
    ///
    /// `sort=updated&direction=desc` is mandatory: with ascending order and `since`,
    /// GitHub walks the *oldest* matching page and stops silently, which reads as a
    /// quiet day. `since` filters on `updated_at`, so an edit to an old comment
    /// resurfaces it and is filtered below.
    pub async fn repo_activity(
        &self,
        owner: &str,
        repo: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<GithubActivity>, GithubError> {
        let since = from.to_rfc3339();
        let full = format!("{owner}/{repo}");

        let issue_path = format!(
            "repos/{full}/issues/comments?since={since}&sort=updated&direction=desc&per_page=100"
        );
        let review_path = format!(
            "repos/{full}/pulls/comments?since={since}&sort=updated&direction=desc&per_page=100"
        );

        let issue_fut = self.get::<Vec<IssueCommentWire>>(&issue_path);
        let review_fut = self.get::<Vec<ReviewCommentWire>>(&review_path);
        let (issue_comments, review_comments) = tokio::try_join!(issue_fut, review_fut)?;

        let mut out: Vec<GithubActivity> = Vec::new();
        // `updated_at` filters here rather than trusting `since`: an edit to a comment
        // from last month matches the server-side window but is not this window's work.
        let in_window = |created: DateTime<Utc>, updated: DateTime<Utc>| {
            created >= from && created <= to || (updated >= from && updated <= to)
        };

        for comment in issue_comments {
            let Some(issue) = comment.issue else { continue };
            if !in_window(comment.created_at, comment.updated_at) {
                continue;
            }
            // Both an issue and a PR's conversation tab come from this endpoint; which
            // one it was does not change what the work was, so both read as MSG.
            let _is_pr = issue.pull_request.is_some();
            out.push(GithubActivity {
                repo: full.clone(),
                number: issue.number,
                kind: ActivityKind::IssueComment,
                title: issue.title.unwrap_or_default(),
                body: comment.body.unwrap_or_default(),
                created_at: comment.created_at,
            });
        }

        for comment in review_comments {
            let Some(pull) = comment.pull_request else {
                continue;
            };
            if !in_window(comment.created_at, comment.updated_at) {
                continue;
            }
            out.push(GithubActivity {
                repo: full.clone(),
                number: pull.number,
                kind: ActivityKind::ReviewComment,
                title: pull.title.unwrap_or_default(),
                body: comment.body.unwrap_or_default(),
                created_at: comment.created_at,
            });
        }

        // Newest first, so a budget trim keeps the most recent work.
        out.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        Ok(out)
    }

    /// Repositories to read, discovered from the user's own recent activity.
    ///
    /// `search/issues` with `author:` finds repos the user opened issues on. It is a
    /// locator here and nothing more — no attempt is made to collect activity from it,
    /// because `commenter:` and `author:` both miss review comments.
    pub async fn discover_repos(&self, since: DateTime<Utc>) -> Result<Vec<String>, GithubError> {
        let query = format!(
            "author:{} author-date:>={}",
            self.login,
            since.format("%Y-%m-%d")
        );
        let encoded = urlencode(&query);
        let path = format!("search/issues?q={encoded}&per_page=30&sort=updated&order=desc");

        #[derive(Deserialize)]
        struct SearchResult {
            #[serde(default)]
            items: Vec<SearchItem>,
        }
        #[derive(Deserialize)]
        struct SearchItem {
            #[serde(default)]
            repository_url: String,
        }

        let result: SearchResult = self.get(&path).await?;
        let mut repos: Vec<String> = result
            .items
            .iter()
            .filter_map(|item| {
                // "https://api.github.com/repos/owner/name" -> "owner/name"
                item.repository_url
                    .rsplit_once("/repos/")
                    .map(|(_, tail)| tail.to_string())
            })
            .collect();
        repos.sort();
        repos.dedup();
        Ok(repos)
    }
}

/// Minimal percent-encoding for a query string. `urlencoding` is not worth a dependency
/// for one call site.
fn urlencode(input: &str) -> String {
    input
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> GithubClient {
        GithubClient::new(Secret::new("ghp_test"), "sanket129").with_base_url(server.uri())
    }

    /// A window covering 2026-10-02 in full, derived from the date rather than a
    /// hand-computed epoch.
    fn window() -> (DateTime<Utc>, DateTime<Utc>) {
        use chrono::{TimeZone, Utc};
        (
            Utc.with_ymd_and_hms(2026, 10, 2, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap(),
        )
    }

    #[tokio::test]
    async fn whoami_returns_the_login() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "login": "sanket129"
            })))
            .mount(&server)
            .await;

        assert_eq!(client(&server).whoami().await.unwrap(), "sanket129");
    }

    #[tokio::test]
    async fn review_comments_are_collected_from_the_pulls_endpoint() {
        // The whole reason this client does not use search: a reviewer's review comments
        // are invisible to `commenter:` and are the bulk of a reviewer's work.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/coot/ai/pulls/comments"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                    "id": 1, "body": "needs a rebase",
                    "created_at": "2026-10-02T09:00:00Z",
                    "updated_at": "2026-10-02T09:00:00Z",
                    "html_url": "https://github.com/coot/ai/pull/482#c1",
                    "pull_request": {"number": 482, "title": "failover jitter"}
                }])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/coot/ai/issues/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let (from, to) = window();
        let activity = client(&server)
            .repo_activity("coot", "ai", from, to)
            .await
            .unwrap();

        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].kind, ActivityKind::ReviewComment);
        assert_eq!(activity[0].number, 482);
    }

    #[tokio::test]
    async fn issue_comments_are_collected_too() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/coot/ai/issues/comments"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                    "id": 2, "body": "blocked on review",
                    "created_at": "2026-10-02T10:00:00Z",
                    "updated_at": "2026-10-02T10:00:00Z",
                    "html_url": "https://github.com/coot/ai/issues/9#c2",
                    "issue": {"number": 9, "title": "indexer chunking"}
                }])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/coot/ai/pulls/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        let (from, to) = window();
        let activity = client(&server)
            .repo_activity("coot", "ai", from, to)
            .await
            .unwrap();

        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].kind, ActivityKind::IssueComment);
    }

    #[tokio::test]
    async fn an_error_body_is_reported_as_an_api_error_not_a_decode_failure() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(401).set_body_json(
                    serde_json::json!({"message": "Bad credentials", "status": 401}),
                ),
            )
            .mount(&server)
            .await;

        // A 404 deserialized as the success type would report "invalid json" and send the
        // user chasing the wrong problem.
        match client(&server).whoami().await {
            Err(GithubError::Api { status, message }) => {
                assert_eq!(status, 401);
                assert_eq!(message, "Bad credentials");
            }
            other => panic!("expected an API error, got {other:?}"),
        }
    }

    #[test]
    fn urlencode_escapes_a_search_query() {
        assert_eq!(
            urlencode("author:sanket129 author-date:>=2026-10-01"),
            "author%3Asanket129+author-date%3A%3E%3D2026-10-01"
        );
    }

    #[tokio::test]
    async fn repo_discovery_normalises_repository_urls() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search/issues"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    {"repository_url": "https://api.github.com/repos/codeacioustech/open-coot-ui"},
                    {"repository_url": "https://api.github.com/repos/codeacioustech/open-coot-ui"},
                    {"repository_url": "https://api.github.com/repos/Sane219/learn_rust"}
                ]
            })))
            .mount(&server)
            .await;

        let repos = client(&server)
            .discover_repos(DateTime::from_timestamp(1_759_300_000, 0).unwrap())
            .await
            .unwrap();

        assert_eq!(
            repos,
            vec![
                "Sane219/learn_rust".to_string(),
                "codeacioustech/open-coot-ui".to_string()
            ],
            "owner/name form, deduped and sorted"
        );
    }
}
