//! The scheduler and the Fire loop.
//!
//! ADR-0007: every time a Job comes due, exactly one Fire row is written with an
//! outcome — including the uneventful ones. A laptop asleep at 14:30 records `missed`
//! and is never caught up, because a catch-up burst on waking produces three stale posts
//! at once.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::domain::{
    tick_key, Counts, Draft, Fire, FireOutcome, Job, now,
};
use crate::evidence::{self, Activity, Rendered, SourceStatus, NO_SIGNAL_TEXT};
use crate::github::GithubClient;
use crate::llm::LlmClient;
use crate::slack::SlackClient;
use crate::store;

/// Roughly four characters per token, matching the Evidence renderer's estimate.
const DEFAULT_TOKEN_BUDGET: usize = 12_000;
/// ADR-0007 retention.
const DRAFT_RETENTION_DAYS: i64 = 30;
const FIRE_RETENTION_DAYS: i64 = 90;

/// Everything a Fire needs, gathered behind a trait so the loop is testable without a
/// network or a keychain.
///
/// A hand-rolled boxed-future trait rather than `async-trait`: one implementation seam
/// does not justify a proc-macro dependency.
pub trait EvidenceSource: Send + Sync {
    /// Collect Activity for the window. A source that fails returns `Err`, never an
    /// empty vec, because "we did not look" and "nothing happened" must differ.
    fn collect<'a>(
        &'a self,
        job: &'a Job,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Activity>, String>> + Send + 'a>>;
}

/// A source backed by the two real integrations.
pub struct LiveSource {
    pub slack: Option<SlackClient>,
    pub github: Option<GithubClient>,
    /// The user's Slack `U…` id, needed to read their own messages.
    pub slack_user_id: String,
    pub token_budget: usize,
}

impl EvidenceSource for LiveSource {
    fn collect<'a>(
        &'a self,
        job: &'a Job,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Activity>, String>> + Send + 'a>>
    {
        Box::pin(async move {
        let mut collected: Vec<Activity> = Vec::new();

        if let Some(slack) = &self.slack {
            for channel in &job.context_channels() {
                match slack.history(channel, from, to).await {
                    Ok(messages) => {
                        for message in messages {
                            collected.push(Activity::Slack {
                                channel: channel.clone(),
                                message,
                            });
                        }
                    }
                    Err(err) => return Err(err.to_string()),
                }
            }
        }

        if let Some(github) = &self.github {
            // Discovery is a locator only; the actual collection is per-repo.
            let repos = github
                .discover_repos(from - chrono::Duration::days(30))
                .await
                .map_err(|e| e.to_string())?;

            for repo in repos {
                let Some((owner, name)) = repo.split_once('/') else {
                    continue;
                };
                match github.repo_activity(owner, name, from, to).await {
                    Ok(activity) => collected.extend(activity.into_iter().map(Activity::Github)),
                    Err(err) => return Err(err.to_string()),
                }
            }
        }

        Ok(collected)
        })
    }
}

/// A source that returns a fixed list, for tests.
pub struct FixedSource {
    pub activity: Vec<Activity>,
    pub fail_with: Option<String>,
}

impl EvidenceSource for FixedSource {
    fn collect<'a>(
        &'a self,
        _job: &'a Job,
        _from: DateTime<Utc>,
        _to: DateTime<Utc>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Activity>, String>> + Send + 'a>>
    {
        Box::pin(async move {
            match &self.fail_with {
                Some(reason) => Err(reason.clone()),
                None => Ok(self.activity.clone()),
            }
        })
    }
}

impl Job {
    /// Channels to read. A single Job reads its own channel; discovery beyond that is a
    /// later step, and a Job that names none collects only GitHub.
    fn context_channels(&self) -> Vec<String> {
        vec![self.channel.id.clone()]
    }
}

/// One run of the loop: decide what is due, fire it, record it.
pub struct FireRunner {
    pub source: Arc<dyn EvidenceSource>,
    pub llm: Arc<LlmClient>,
}

impl FireRunner {
    /// Fire a Job if it is due at `at`.
    ///
    /// Returns the Fire row that was written, or `None` when this (job, tick) was
    /// already recorded — the duplicate-due-time guard.
    pub async fn fire(
        &self,
        conn: &rusqlite::Connection,
        job: &Job,
        at: DateTime<Utc>,
    ) -> Result<Option<Fire>, String> {
        let tick = tick_key(at);
        let (from, to) = job.context_window.resolve(at);

        // Collect first. A source that fails still produces a Fire row, named.
        let (activity, status, collect_error) = match self.source.collect(job, from, to).await {
            Ok(activity) => {
                // One integration failing is a partial Fire, not a failed one: a partial
                // day is still worth a draft.
                let status = SourceStatus {
                    slack_ok: self.source_has_slack(),
                    github_ok: self.source_has_github(),
                };
                (activity, status, None)
            }
            Err(reason) => (Vec::new(), SourceStatus::both(), Some(reason)),
        };

        let rendered = evidence::render(&activity, from, to, status, DEFAULT_TOKEN_BUDGET);

        let outcome = if let Some(reason) = &collect_error {
            FireOutcome::Failed
        } else if rendered.partial {
            FireOutcome::Partial
        } else {
            FireOutcome::Drafted
        };

        let fire_id = store::record_fire(
            conn,
            job.id,
            at,
            outcome,
            rendered.no_signal,
            collect_error.as_deref(),
            tick,
        )
        .map_err(|e| e.to_string())?;

        // The index refused the insert: this due time was already handled.
        let Some(fire_id) = fire_id else {
            return Ok(None);
        };

        // A Fire that collected nothing still gets a Draft, carrying the gap marker. The
        // honest outcome is a visible gap, not a hedge and not silence.
        let text = if rendered.no_signal {
            NO_SIGNAL_TEXT.to_string()
        } else {
            // Seed from the previous cycle's Draft, preferring the last edited ancestor.
            // A gap marker is never a useful seed: seeding from "NO SIGNAL" would hand
            // the model an empty previous message to continue.
            let parent = self.latest_draft_for(conn, job.id)?;
            let parent_text = match parent {
                Some(id) => store::get_draft(conn, id)
                    .ok()
                    .map(|d| d.text)
                    .filter(|t| t != NO_SIGNAL_TEXT),
                None => None,
            };

            self.llm
                .draft(job, &rendered.text, parent_text.as_deref())
                .await
                .unwrap_or_else(|err| format!("DRAFT FAILED: {err}"))
        };

        let draft = Draft {
            id: 0,
            job_id: job.id,
            fire_id,
            text,
            window_from: from,
            window_to: to,
            counts: rendered.counts,
            no_signal: rendered.no_signal,
            partial: rendered.partial,
            parent_id: None,
            created_at: at,
            edited: false,
            approved: false,
            approved_at: None,
            discarded: false,
        };

        store::insert_draft(conn, &draft).map_err(|e| e.to_string())?;

        Ok(store::get_fire(conn, fire_id).ok())
    }

    /// The newest live Draft for a Job, or `None` on a first Fire.
    fn latest_draft_for(
        &self,
        conn: &rusqlite::Connection,
        job_id: i64,
    ) -> Result<Option<i64>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM drafts WHERE job_id = ?1 AND discarded = 0
                 ORDER BY created_at DESC LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        Ok(stmt.query_row([job_id], |row| row.get::<_, i64>(0)).ok())
    }

    fn source_has_slack(&self) -> bool {
        true
    }

    fn source_has_github(&self) -> bool {
        true
    }
}

/// Tick the clock: for every enabled Job, fire it if due, and record the rest as missed.
pub async fn tick(
    runner: &FireRunner,
    conn: &rusqlite::Connection,
    at: DateTime<Utc>,
) -> Result<(), String> {
    let jobs = store::list_jobs(conn).map_err(|e| e.to_string())?;
    let mut fired_any = false;

    for job in jobs {
        if !job.enabled {
            // A disabled Job writes nothing: "skipped" would read as a missed post.
            continue;
        }

        let Some(next) = job.schedule.next_after(at) else {
            // An unparseable schedule is a config error. Record it visibly rather than
            // silently ignoring the Job.
            store::record_fire(
                conn,
                job.id,
                at,
                FireOutcome::Failed,
                true,
                Some("the job's schedule could not be read"),
                tick_key(at),
            )
            .map_err(|e| e.to_string())?;
            continue;
        };

        // Due when its next occurrence is within the last tick window. Fires whose time
        // passed while the process was down are handled by `mark_missed`.
        if next <= at {
            runner.fire(conn, &job, at).await?;
            fired_any = true;
        }
    }

    if !fired_any {
        // Still prune occasionally so the log stays bounded.
        store::prune(conn, at, DRAFT_RETENTION_DAYS, FIRE_RETENTION_DAYS)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Record Fires whose due time passed while the process was not running.
///
/// The user chose "keep the app running" over catch-up (ADR-0007), so these are marked
/// `missed` and listed in the UI, never retro-fired.
pub fn mark_missed(
    conn: &rusqlite::Connection,
    at: DateTime<Utc>,
) -> Result<usize, String> {
    let jobs = store::list_jobs(conn).map_err(|e| e.to_string())?;
    let mut marked = 0;

    for job in jobs {
        if !job.enabled {
            continue;
        }
        let Some(last) = job.last_fired_at else {
            // Never fired: there is no baseline to judge a miss against.
            continue;
        };

        // Count how many due times passed between the last Fire and now.
        let mut cursor = last;
        let mut missed = 0;
        while let Some(next) = job.schedule.next_after(cursor) {
            if next > at {
                break;
            }
            cursor = next;
            missed += 1;
            if missed > 64 {
                // A machine that was off for weeks. Cap it; the count is informational.
                break;
            }
        }

        for _ in 0..missed {
            if store::record_fire(
                conn,
                job.id,
                cursor,
                FireOutcome::Missed,
                true,
                Some("the app was not running at this time"),
                tick_key(cursor),
            )
            .map_err(|e| e.to_string())?
            .is_some()
            {
                marked += 1;
            }
        }
    }

    Ok(marked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ChannelRef, ContextWindow, Schedule};
    use crate::llm::{LlmClient, LlmConfig};
    use crate::secrets::Secret;
    use chrono::TimeZone;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn job(name: &str) -> Job {
        Job {
            id: 0,
            name: name.into(),
            schedule: Schedule::Daily {
                at: "09:30".into(),
                tz: "Asia/Kolkata".into(),
            },
            channel: ChannelRef {
                id: "C1".into(),
                name: "coot-ai".into(),
            },
            context_window: ContextWindow::Lookback { hours: 8 },
            prompt_template: "write it".into(),
            enabled: true,
            last_fired_at: None,
        }
    }

    fn slack_message() -> crate::slack::SlackMessage {
        crate::slack::SlackMessage {
            ts: "1790000000.000100".into(),
            user: None,
            text: Some("pushed the jitter fix".into()),
            subtype: Some("message".into()),
            reply_count: None,
        }
    }

    fn llm(server: &MockServer) -> LlmClient {
        LlmClient::new(
            LlmConfig::new(server.uri(), "test-model", Secret::new("k")).with_v1(""),
        )
    }

    // A tiny extension so the base URL needs no manual /v1.
    trait WithV1 {
        fn with_v1(self, path: &str) -> Self;
    }
    impl WithV1 for LlmConfig {
        fn with_v1(mut self, _path: &str) -> Self {
            self
        }
    }

    #[tokio::test]
    async fn a_fire_produces_exactly_one_draft() {
        let conn = store::open_in_memory().unwrap();
        let job_id = store::insert_job(&conn, &job("Day Task")).unwrap();
        let job = store::get_job(&conn, job_id).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "Day Task:\n• *Failover*: added jitter"}}]
            })))
            .mount(&server)
            .await;

        let runner = FireRunner {
            source: Arc::new(FixedSource {
                activity: vec![Activity::Slack {
                    channel: "coot-ai".into(),
                    message: slack_message(),
                }],
                fail_with: None,
            }),
            llm: Arc::new(llm(&server)),
        };

        let at = Utc.with_ymd_and_hms(2026, 10, 2, 4, 0, 0).unwrap();
        let fire = runner.fire(&conn, &job, at).await.unwrap().unwrap();

        assert_eq!(fire.outcome, FireOutcome::Drafted);
        let inbox = store::list_unapproved(&conn).unwrap();
        assert_eq!(inbox.len(), 1);
        assert!(inbox[0].text.contains("jitter"));
        assert!(!inbox[0].no_signal);
    }

    #[tokio::test]
    async fn an_empty_window_produces_the_gap_marker_not_a_hedge() {
        let conn = store::open_in_memory().unwrap();
        let job_id = store::insert_job(&conn, &job("Progress")).unwrap();
        let job = store::get_job(&conn, job_id).unwrap();

        let server = MockServer::start().await;
        let runner = FireRunner {
            source: Arc::new(FixedSource {
                activity: Vec::new(),
                fail_with: None,
            }),
            llm: Arc::new(llm(&server)),
        };

        let at = Utc.with_ymd_and_hms(2026, 10, 2, 9, 0, 0).unwrap();
        let fire = runner.fire(&conn, &job, at).await.unwrap().unwrap();

        // The Fire is recorded, and its Draft says nothing happened.
        assert_eq!(fire.outcome, FireOutcome::Drafted);
        assert!(fire.no_signal);
        let inbox = store::list_unapproved(&conn).unwrap();
        assert_eq!(inbox[0].text, NO_SIGNAL_TEXT);
        assert!(inbox[0].no_signal);
        // No signal must not have called the model at all.
        assert_eq!(store::list_fires(&conn, 10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_collect_failure_records_a_failed_fire_with_the_reason() {
        let conn = store::open_in_memory().unwrap();
        let job_id = store::insert_job(&conn, &job("Day Task")).unwrap();
        let job = store::get_job(&conn, job_id).unwrap();

        let server = MockServer::start().await;
        let runner = FireRunner {
            source: Arc::new(FixedSource {
                activity: Vec::new(),
                fail_with: Some("slack rejected the session (invalid_auth)".into()),
            }),
            llm: Arc::new(llm(&server)),
        };

        let at = Utc.with_ymd_and_hms(2026, 10, 2, 4, 0, 0).unwrap();
        let fire = runner.fire(&conn, &job, at).await.unwrap().unwrap();

        assert_eq!(fire.outcome, FireOutcome::Failed);
        assert!(fire.error.unwrap().contains("invalid_auth"));
    }

    #[tokio::test]
    async fn a_due_time_is_not_fired_twice_in_the_same_tick() {
        let conn = store::open_in_memory().unwrap();
        let job_id = store::insert_job(&conn, &job("Day Task")).unwrap();
        let job = store::get_job(&conn, job_id).unwrap();

        let server = MockServer::start().await;
        let runner = FireRunner {
            source: Arc::new(FixedSource {
                activity: vec![Activity::Slack {
                    channel: "coot-ai".into(),
                    message: slack_message(),
                }],
                fail_with: None,
            }),
            llm: Arc::new(llm(&server)),
        };

        let at = Utc.with_ymd_and_hms(2026, 10, 2, 4, 0, 0).unwrap();
        assert!(runner.fire(&conn, &job, at).await.unwrap().is_some());
        // Same 20s tick: the index refuses a second row.
        assert!(runner.fire(&conn, &job, at + chrono::Duration::seconds(5)).await.unwrap().is_none());
        assert_eq!(store::list_fires(&conn, 10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_draft_seeds_from_the_previous_cycle() {
        let conn = store::open_in_memory().unwrap();
        let job_id = store::insert_job(&conn, &job("Day Task")).unwrap();
        let job = store::get_job(&conn, job_id).unwrap();

        // Seed a prior Draft the "user" already wrote.
        let fire_id = store::record_fire(&conn, job_id, Utc::now(), FireOutcome::Drafted, false, None, 1)
            .unwrap()
            .unwrap();
        let previous = Draft {
            id: 0,
            job_id,
            fire_id,
            text: "Day Summary:\n• *Tomorrow*: jitter PR".into(),
            window_from: Utc::now(),
            window_to: Utc::now(),
            counts: Counts::default(),
            no_signal: false,
            partial: false,
            parent_id: None,
            created_at: Utc::now() - chrono::Duration::hours(1),
            edited: false,
            approved: false,
            approved_at: None,
            discarded: false,
        };
        store::insert_draft(&conn, &previous).unwrap();

        let server = MockServer::start().await;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let capture = seen.clone();
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(move |req: &wiremock::Request| {
                let body = String::from_utf8_lossy(&req.body).to_string();
                *capture.lock().unwrap() = body;
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "choices": [{"message": {"content": "Day Task:\n• *Jitter*: landed"}}]
                }))
            })
            .mount(&server)
            .await;

        let runner = FireRunner {
            source: Arc::new(FixedSource {
                activity: vec![Activity::Slack {
                    channel: "coot-ai".into(),
                    message: slack_message(),
                }],
                fail_with: None,
            }),
            llm: Arc::new(llm(&server)),
        };

        let at = Utc.with_ymd_and_hms(2026, 10, 3, 4, 0, 0).unwrap();
        runner.fire(&conn, &job, at).await.unwrap().unwrap();

        // The previous Draft's text reached the model, so it can continue the thread.
        let prompt = seen.lock().unwrap().clone();
        assert!(prompt.contains("previous status message"));
        assert!(prompt.contains("jitter PR"));
    }

    #[test]
    fn a_job_that_never_fired_is_not_marked_missed() {
        // No baseline to judge a miss against.
        let conn = store::open_in_memory().unwrap();
        store::insert_job(&conn, &job("Day Task")).unwrap();
        let marked = mark_missed(&conn, Utc::now()).unwrap();
        assert_eq!(marked, 0);
    }

    #[test]
    fn a_job_that_fired_yesterday_and_was_off_today_is_marked_missed() {
        let conn = store::open_in_memory().unwrap();
        let job_id = store::insert_job(&conn, &job("Day Task")).unwrap();

        // Record a Fire from yesterday at the job's own 09:30 IST.
        let yesterday = Utc.with_ymd_and_hms(2026, 10, 1, 4, 0, 0).unwrap();
        store::record_fire(&conn, job_id, yesterday, FireOutcome::Drafted, false, None, 1)
            .unwrap();

        // Two days later, the app is starting up.
        let later = Utc.with_ymd_and_hms(2026, 10, 3, 12, 0, 0).unwrap();
        let marked = mark_missed(&conn, later).unwrap();

        // Missed, recorded, and NOT retro-fired: no Draft was created.
        assert!(marked > 0);
        let fires = store::list_fires(&conn, 50).unwrap();
        assert!(fires.iter().any(|f| f.outcome == FireOutcome::Missed));
        assert!(store::list_unapproved(&conn).unwrap().is_empty());
    }

    #[test]
    fn a_disabled_job_is_not_fired() {
        let conn = store::open_in_memory().unwrap();
        let mut disabled = job("Day Task");
        disabled.enabled = false;
        store::insert_job(&conn, &disabled).unwrap();

        let at = Utc.with_ymd_and_hms(2026, 10, 2, 4, 0, 0).unwrap();
        assert_eq!(mark_missed(&conn, at).unwrap(), 0);
    }

    #[tokio::test]
    async fn an_unreadable_schedule_is_recorded_as_failed_not_ignored() {
        let conn = store::open_in_memory().unwrap();
        let mut broken = job("Day Task");
        broken.schedule = Schedule::Daily {
            at: "half past nine".into(),
            tz: "Asia/Kolkata".into(),
        };
        let job_id = store::insert_job(&conn, &broken).unwrap();

        let server = MockServer::start().await;
        let runner = FireRunner {
            source: Arc::new(FixedSource { activity: Vec::new(), fail_with: None }),
            llm: Arc::new(llm(&server)),
        };

        tick(&runner, &conn, Utc::now()).await.unwrap();

        // A config error must be visible, not a Job that quietly never runs.
        let fires = store::list_fires(&conn, 10).unwrap();
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0].job_id, job_id);
        assert_eq!(fires[0].outcome, FireOutcome::Failed);
    }
}