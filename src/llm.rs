//! LLM client and the two roles.
//!
//! The roles are separate contracts, not two prompts for one function. The Plan Role
//! returns strict JSON and has no ability to send anything; the Draft Role returns Slack
//! mrkdwn and has no authority over schedules. Neither can post — only `approve_draft`
//! reaches Slack, and that is in the web layer.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::Job;
use crate::evidence::{draft_instructions, NO_SIGNAL_TEXT};
use crate::secrets::Secret;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("no model configured")]
    NotConfigured,
    #[error("llm returned {status}: {body}")]
    Api { status: u16, body: String },
    #[error("llm response was not valid json: {0}")]
    Decode(String),
    /// The Plan Role's output did not fit the schema. Shown raw rather than repaired:
    /// auto-fixing a malformed schedule is how a bad config becomes a working one.
    #[error("the model proposed something this tool cannot use: {detail}")]
    UnusablePlan { detail: String, raw: String },
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
}

/// Which OpenAI-compatible endpoint and model to use.
///
/// The model is chosen explicitly, never discovered or silently substituted: a wrong
/// model at a 2pm deadline is worse than a loud error.
#[derive(Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    api_key: Secret,
}

impl std::fmt::Debug for LlmConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmConfig")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl LlmConfig {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>, api_key: Secret) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            api_key,
        }
    }
}

pub struct LlmClient {
    http: reqwest::Client,
    config: LlmConfig,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    /// Low, because these are factual posts about a real day.
    temperature: f32,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: Option<String>,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            config,
        }
    }

    pub fn config(&self) -> &LlmConfig {
        &self.config
    }

    async fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        let body = ChatRequest {
            model: &self.config.model,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: system,
                },
                ChatMessage {
                    role: "user",
                    content: user,
                },
            ],
            temperature: 0.3,
        };

        let text = self
            .http
            .post(format!("{}/chat/completions", self.config.base_url))
            .bearer_auth(self.config.api_key.expose())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_string(&body).unwrap_or_default())
            .send()
            .await?
            .text()
            .await?;

        // Check for an error body before deserializing: an error payload and a success
        // payload are both JSON, and parsing the wrong one reports a decode failure
        // instead of the real problem.
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(err) = value.get("error") {
                let message = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                return Err(LlmError::Api {
                    status: err
                        .get("code")
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0) as u16,
                    body: message,
                });
            }
        }

        let response: ChatResponse =
            serde_json::from_str(&text).map_err(|e| LlmError::Decode(e.to_string()))?;

        response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or_else(|| LlmError::Decode("no content in the response".into()))
    }

    /// `GET /v1/models` — a cheap probe so a bad base URL or key fails at setup rather
    /// than at 14:30 with a window to cover.
    pub async fn probe(&self) -> Result<Vec<String>, LlmError> {
        #[derive(Deserialize)]
        struct ModelsResponse {
            #[serde(default)]
            data: Vec<ModelEntry>,
        }
        #[derive(Deserialize)]
        struct ModelEntry {
            id: String,
        }

        let text = self
            .http
            .get(format!("{}/models", self.config.base_url))
            .bearer_auth(self.config.api_key.expose())
            .send()
            .await?
            .text()
            .await?;

        let response: ModelsResponse =
            serde_json::from_str(&text).map_err(|e| LlmError::Decode(e.to_string()))?;
        Ok(response.data.into_iter().map(|m| m.id).collect())
    }

    // ── Plan Role ───────────────────────────────────────────────────────────

    /// Turn a plain-English description of someone's routine into proposed Jobs.
    ///
    /// This proposes. Nothing is scheduled until the user confirms the rendered
    /// preview (ADR-0004 means the model may not name anything on its own authority).
    pub async fn plan(
        &self,
        description: &str,
    ) -> Result<Vec<PlannedJob>, LlmError> {
        let schema = "{\"jobs\":[{\"name\":\"...\",\"at\":\"HH:MM 24-hour\",\
                       \"tz\":\"IANA timezone\",\"channel\":\"#name\",\
                       \"context\":\"lookback|since\",\"since_at\":\"HH:MM\",\
                       \"previous_day\":true,\"prompt\":\"instructions\"}]}";

        let system = format!(
            "You turn a description of someone's recurring work-status routine into \
             scheduled jobs.\n\
             Reply with JSON only, no prose and no code fence:\n\
             {schema}\n\
             Rules:\n\
             - Use the timezone the user names, or ask for it if they did not.\n\
             - Keep each prompt in that team's own words and house style. Invent no \
             message names, formats, or conventions the user did not describe.\n\
             - A post about intentions in the morning should read the previous \
             evening's context: use context \"since\" with previous_day true.\n\
             - A post about completed work should use context \"lookback\"."
        );

        let raw = self.complete(&system, description).await?;
        parse_plan(&raw)
    }

    // ── Draft Role ──────────────────────────────────────────────────────────

    /// Turn Evidence into a Draft.
    ///
    /// Returns the gap marker rather than prose when there is no evidence, so an empty
    /// day produces a visible gap instead of a confident invention.
    pub async fn draft(
        &self,
        job: &Job,
        evidence: &str,
        parent_text: Option<&str>,
    ) -> Result<String, LlmError> {
        let mut system = String::new();
        system.push_str("You write short work-status messages for Slack.\n\n");
        system.push_str(&job.prompt_template);
        system.push_str("\n\n");
        system.push_str(draft_instructions());

        let mut user = String::new();
        if let Some(previous) = parent_text {
            // The morning post is seeded from the previous cycle's Draft. Handed over as
            // context the user already wrote, so the model continues their judgement
            // rather than re-deriving it.
            user.push_str("## Your previous status message\n");
            user.push_str(previous);
            user.push_str("\n\n");
        }
        user.push_str(evidence);

        let raw = self.complete(&system, &user).await?;
        Ok(clean_draft(&raw))
    }
}

/// Strip the shapes a model adds that a Slack post must not contain.
fn clean_draft(raw: &str) -> String {
    let mut text = raw.trim().to_string();

    // Remove a wrapping code fence, which a model adds reflexively and which would post
    // literally.
    if text.starts_with("```") {
        let mut lines = text.lines();
        lines.next();
        if let Some(last) = lines.next_back() {
            if last.trim().starts_with("```") {
                text = lines.collect::<Vec<_>>().join("\n");
            }
        }
    }

    text.trim().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedJob {
    pub name: String,
    pub at: String,
    pub tz: String,
    pub channel: String,
    pub context: String,
    #[serde(default)]
    pub since_at: Option<String>,
    #[serde(default)]
    pub previous_day: bool,
    pub prompt: String,
}

impl PlannedJob {
    /// Every problem with this proposal, not just the first.
    ///
    /// A user fixing their description wants the whole list at once. Returning early
    /// would make them resubmit three times to learn three things.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();

        if self.name.trim().is_empty() {
            out.push("a job has no name".into());
        }
        if self.at.parse::<chrono::NaiveTime>().is_err() {
            out.push(format!("{:?} is not a HH:MM time", self.at));
        }
        if self.tz.parse::<chrono_tz::Tz>().is_err() {
            out.push(format!("{:?} is not a known IANA timezone", self.tz));
        }
        if self.channel.trim().is_empty() {
            out.push("a job has no channel".into());
        }
        match self.context.as_str() {
            "lookback" => {}
            "since" => match self.since_at.as_deref() {
                None => out.push("context \"since\" needs since_at".into()),
                Some(at) if at.parse::<chrono::NaiveTime>().is_err() => {
                    out.push(format!("{at:?} is not a HH:MM time"))
                }
                Some(_) => {}
            },
            other => out.push(format!("context {other:?} is not lookback or since")),
        }

        out
    }

    /// How long a lookback Job should reach back, when the model did not say.
    pub fn lookback_hours(&self) -> i64 {
        8
    }
}

/// Parse the Plan Role's reply. A model that wraps JSON in prose is a common failure, so
/// the outermost braces are located rather than demanding a clean body.
fn parse_plan(raw: &str) -> Result<Vec<PlannedJob>, LlmError> {
    let body = raw
        .trim()
        .strip_prefix("```json")
        .or_else(|| raw.trim().strip_prefix("```"))
        .map(str::trim)
        .unwrap_or(raw.trim())
        .trim_end_matches("```")
        .trim();

    let json = match body.find('{') {
        Some(start) => {
            let end = body.rfind('}').unwrap_or(body.len());
            &body[start..=end]
        }
        None => body,
    };

    #[derive(Deserialize)]
    struct Plan {
        #[serde(default)]
        jobs: Vec<PlannedJob>,
    }

    let plan: Plan = serde_json::from_str(json)
        .map_err(|e| LlmError::UnusablePlan {
            detail: e.to_string(),
            raw: raw.to_string(),
        })?;

    if plan.jobs.is_empty() {
        return Err(LlmError::UnusablePlan {
            detail: "the proposal contained no jobs".into(),
            raw: raw.to_string(),
        });
    }

    // Validate before returning. Every problem is reported, not just the first, so a
    // user fixing their description sees all of it at once.
    let problems: Vec<String> = plan.jobs.iter().flat_map(|j| j.problems()).collect();

    if !problems.is_empty() {
        return Err(LlmError::UnusablePlan {
            detail: problems.join("; "),
            raw: raw.to_string(),
        });
    }

    Ok(plan.jobs)
}

/// When a Fire's window closed, for display.
pub fn window_label(from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    format!("{} → {}", from.format("%d %b %H:%M"), to.format("%d %b %H:%M"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LlmConfig {
        LlmConfig::new("http://localhost:9/v1", "test-model", Secret::new("k"))
    }

    fn client() -> LlmClient {
        LlmClient::new(config())
    }

    fn valid_plan_json() -> &'static str {
        "{\"jobs\":[{\"name\":\"Day Task\",\"at\":\"09:30\",\"tz\":\"Asia/Kolkata\",\
         \"channel\":\"#coot-ai\",\"context\":\"since\",\"since_at\":\"18:30\",\
         \"previous_day\":true,\"prompt\":\"write a day task\"}]}"
    }

    #[test]
    fn a_clean_plan_parses() {
        let jobs = parse_plan(valid_plan_json()).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].at, "09:30");
        assert!(jobs[0].previous_day);
    }

    #[test]
    fn a_fenced_plan_parses() {
        // Models wrap JSON reflexively. Refusing this would make the Plan Role useless.
        let fenced = format!("```json\n{}\n```", valid_plan_json());
        assert_eq!(parse_plan(&fenced).unwrap().len(), 1);
    }

    #[test]
    fn a_plan_wrapped_in_prose_still_parses() {
        let chatty = format!(
            "Here are the jobs I found:\n{}\nLet me know if you want different times.",
            valid_plan_json()
        );
        assert_eq!(parse_plan(&chatty).unwrap().len(), 1);
    }

    #[test]
    fn an_unparseable_plan_is_reported_with_its_raw_text() {
        // Never auto-repaired: a repaired schedule is one the user never chose.
        match parse_plan("I could not understand that.") {
            Err(LlmError::UnusablePlan { raw, .. }) => {
                assert!(raw.contains("could not understand"))
            }
            other => panic!("expected UnusablePlan, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_proposal_is_refused() {
        assert!(matches!(
            parse_plan("{\"jobs\":[]}"),
            Err(LlmError::UnusablePlan { .. })
        ));
    }

    #[test]
    fn every_problem_is_reported_not_just_the_first() {
        let bad = "{\"jobs\":[{\"name\":\"A\",\"at\":\"half nine\",\"tz\":\"Mars/Olympus\",\
                    \"channel\":\"#c\",\"context\":\"since\",\"since_at\":\"18:30\",\"prompt\":\"p\"},\
                   {\"name\":\"\",\"at\":\"09:30\",\"tz\":\"Asia/Kolkata\",\
                    \"channel\":\"#c\",\"context\":\"lookback\",\"prompt\":\"p\"}]}";
        match parse_plan(bad) {
            Err(LlmError::UnusablePlan { detail, .. }) => {
                assert!(detail.contains("HH:MM"));
                assert!(detail.contains("timezone"));
                assert!(detail.contains("no name"));
            }
            other => panic!("expected UnusablePlan, got {other:?}"),
        }
    }

    #[test]
    fn since_context_requires_a_time() {
        let bad = "{\"jobs\":[{\"name\":\"A\",\"at\":\"09:30\",\"tz\":\"Asia/Kolkata\",\
                    \"channel\":\"#c\",\"context\":\"since\",\"prompt\":\"p\"}]}";
        match parse_plan(bad) {
            Err(LlmError::UnusablePlan { detail, .. }) => assert!(detail.contains("since_at")),
            other => panic!("expected UnusablePlan, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_context_is_refused() {
        let bad = "{\"jobs\":[{\"name\":\"A\",\"at\":\"09:30\",\"tz\":\"Asia/Kolkata\",\
                    \"channel\":\"#c\",\"context\":\"last_five_minutes\",\"prompt\":\"p\"}]}";
        assert!(parse_plan(bad).is_err());
    }

    #[test]
    fn debug_on_the_config_never_prints_the_key() {
        let rendered = format!("{:?}", config());
        assert!(!rendered.contains("k\""));
        assert!(rendered.contains("redacted"));
    }

    #[test]
    fn a_wrapped_code_fence_is_removed_from_a_draft() {
        // A fence would post to Slack literally.
        assert_eq!(clean_draft("```\nDay Task:\n• *x*: y\n```"), "Day Task:\n• *x*: y");
        assert_eq!(clean_draft("```markdown\nDay Task:\n```"), "Day Task:");
    }

    #[test]
    fn a_plain_draft_is_left_alone() {
        let text = "Day Task:\n• *Failover*: added jitter";
        assert_eq!(clean_draft(text), text);
    }

    #[test]
    fn instructions_pin_slack_bold_and_the_gap_marker() {
        let instructions = draft_instructions();
        assert!(instructions.contains("*bold*, never **bold**"));
        assert!(instructions.contains(NO_SIGNAL_TEXT));
    }
}