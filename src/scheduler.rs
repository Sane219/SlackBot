//! The scheduler and the Fire loop.
//!
//! ADR-0007: every time a Job comes due, exactly one Fire row is written with an
//! outcome — including the uneventful ones. A laptop asleep at 14:30 records `missed`
//! and is never caught up, because a catch-up burst on waking produces three stale posts
//! at once.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::domain::{tick_key, Draft, Fire, FireOutcome, Job};
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
/// does not justify a proc-macro dependency. The Job is taken **by value** so the future
/// borrows nothing from the caller and is therefore `Send + 'static`, which is what axum
/// requires of a handler's future.
pub trait EvidenceSource: Send + Sync {
    /// Collect Activity for the window. A source that fails returns `Err`, never an
    /// empty vec, because "we did not look" and "nothing happened" must differ.
    fn collect(
        &self,
        job: Job,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Activity>, String>> + Send>>;
}

/// A source backed by the two real integrations.
#[derive(Clone)]
pub struct LiveSource {
    pub slack: Option<SlackClient>,
    pub github: Option<GithubClient>,
}

impl EvidenceSource for LiveSource {
    fn collect(
        &self,
        job: Job,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Activity>, String>> + Send>>
    {
        // Cloning the Arc and moving it into the async block is what makes the future
        // owned rather than borrowing `&self`, which is what axum needs in order to call
        // this from a handler.
        let me = self.clone();
        Box::pin(async move {
            let mut collected: Vec<Activity> = Vec::new();

            if let Some(slack) = &me.slack {
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

            if let Some(github) = &me.github {
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
                        Ok(activity) => {
                            collected.extend(activity.into_iter().map(Activity::Github))
                        }
                        Err(err) => return Err(err.to_string()),
                    }
                }
            }

            Ok(collected)
        })
    }
}

/// A source that returns a fixed list, for tests.
#[cfg(test)]
#[derive(Clone)]
pub struct FixedSource {
    pub activity: Vec<Activity>,
    pub fail_with: Option<String>,
}

#[cfg(test)]
impl EvidenceSource for FixedSource {
    fn collect(
        &self,
        _job: Job,
        _from: DateTime<Utc>,
        _to: DateTime<Utc>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Activity>, String>> + Send>>
    {
        let me = self.clone();
        Box::pin(async move {
            match &me.fail_with {
                Some(reason) => Err(reason.clone()),
                None => Ok(me.activity.clone()),
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

/// What a Fire collected, before anything is written. The outcome is decided here so a
/// collect failure is a `failed` Fire rather than a silent gap.
pub struct Collected {
    pub rendered: Rendered,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub outcome: FireOutcome,
    pub error: Option<String>,
}

/// One run of the loop: decide what is due, fire it, record it.
pub struct FireRunner {
    pub source: Arc<dyn EvidenceSource>,
    pub llm: Arc<LlmClient>,
}

impl FireRunner {
    /// Fire a Job now: collect, render, record, draft.
    ///
    /// Takes the database lock by path and takes it in short steps, because a Fire makes
    /// network calls and a guard held across an await would be neither `Send` nor fair to
    /// the HTTP handlers sharing the same database.
    pub async fn fire(
        &self,
        conn: &std::sync::Mutex<rusqlite::Connection>,
        job: &Job,
        at: DateTime<Utc>,
    ) -> Result<Option<Fire>, String> {
        // Collect with nothing locked: this is the slow part.
        let collected = self.collect_for_fire(job, at).await;

        let (fire_id, seed) = {
            let guard = conn.lock().unwrap_or_else(|e| e.into_inner());
            let fire_id = self.record(&guard, job.id, at, &collected)?;
            let seed = self.seed_text(&guard, job.id).unwrap_or(None);
            (fire_id, seed)
        };

        let Some(fire_id) = fire_id else {
            return Ok(None);
        };

        let text = self.compose_text(job, &collected, seed).await;

        let fire = {
            let guard = conn.lock().unwrap_or_else(|e| e.into_inner());
            self.write_draft(
                &guard,
                job,
                fire_id,
                collected.rendered,
                collected.from,
                collected.to,
                at,
                text,
            )?;
            store::get_fire(&guard, fire_id).map_err(|e| e.to_string())?
        };

        Ok(Some(fire))
    }

    /// Collect, render and record the Fire. No draft is written yet.
    ///
    /// Split out so the caller can release the database lock before the model is called:
    /// a Fire's network work must not block every other request.
    /// Collect and render, but write nothing. Takes no connection: the network work
    /// happens here, and holding the database lock across it would block every other
    /// request for the seconds a Fire takes.
    pub async fn collect_for_fire(&self, job: &Job, at: DateTime<Utc>) -> Collected {
        let (from, to) = job.context_window.resolve(at);

        let (activity, status, error) = match self.source.collect(job.clone(), from, to).await {
            Ok(activity) => {
                // One integration failing is a partial Fire, not a failed one: a partial
                // day is still worth a draft.
                let status = SourceStatus {
                    slack_ok: true,
                    github_ok: true,
                };
                (activity, status, None)
            }
            Err(reason) => (Vec::new(), SourceStatus::both(), Some(reason)),
        };

        let rendered = evidence::render(&activity, from, to, status, DEFAULT_TOKEN_BUDGET);
        let outcome = if error.is_some() {
            FireOutcome::Failed
        } else if rendered.partial {
            FireOutcome::Partial
        } else {
            FireOutcome::Drafted
        };

        Collected {
            rendered,
            from,
            to,
            outcome,
            error,
        }
    }

    /// Record a Fire that has been collected. Synchronous: no network, so holding the
    /// connection is free.
    pub fn record(
        &self,
        conn: &rusqlite::Connection,
        job_id: i64,
        at: DateTime<Utc>,
        collected: &Collected,
    ) -> Result<Option<i64>, String> {
        store::record_fire(
            conn,
            job_id,
            at,
            collected.outcome,
            collected.rendered.no_signal,
            collected.error.as_deref(),
            tick_key(at),
        )
        .map_err(|e| e.to_string())
    }

    /// Compose the Draft's text. Async and lock-free: the model call happens here, then
    /// the caller writes the result in one short step.
    pub async fn compose_text(
        &self,
        job: &Job,
        collected: &Collected,
        seed: Option<String>,
    ) -> String {
        // A Fire that collected nothing never calls the model: the gap marker is a
        // decision, not a prompt.
        if collected.rendered.no_signal {
            return NO_SIGNAL_TEXT.to_string();
        }

        self.llm
            .draft(job, &collected.rendered.text, seed.as_deref())
            .await
            .unwrap_or_else(|err| format!("DRAFT FAILED: {err}"))
    }

    /// The seed text for a Draft: the previous cycle's Draft, or none.
    ///
    /// Synchronous so the caller can read it before releasing the lock for the model call.
    pub fn seed_text(
        &self,
        conn: &rusqlite::Connection,
        job_id: i64,
    ) -> Result<Option<String>, String> {
        let parent = self.latest_draft_for(conn, job_id)?;
        // A gap marker is never a useful seed: seeding from "NO SIGNAL" would hand the
        // model an empty previous message to continue.
        Ok(match parent {
            Some(id) => store::get_draft(conn, id)
                .ok()
                .map(|d| d.text)
                .filter(|t| t != NO_SIGNAL_TEXT),
            None => None,
        })
    }

    /// Write the Draft for a recorded Fire. Synchronous: the model has already been
    /// called by the caller, so no lock is held across an await.
    #[allow(clippy::too_many_arguments)]
    pub fn write_draft(
        &self,
        conn: &rusqlite::Connection,
        job: &Job,
        fire_id: i64,
        rendered: Rendered,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        at: DateTime<Utc>,
        text: String,
    ) -> Result<(), String> {
        // A Fire that collected nothing still gets a Draft, carrying the gap marker. The
        // text arrives already composed so no model call happens under the lock.
        let text = if rendered.no_signal {
            NO_SIGNAL_TEXT.to_string()
        } else {
            text
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
        Ok(())
    }

    pub fn latest_draft_for(
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
}

/// The moment a Job was most recently due, if that moment is within the catch-up window.
///
/// Returns `None` when the Job is not due yet. Capped at one window back, so a machine
/// that was off for a week records the misses through `mark_missed` rather than firing a
/// week of stale posts on wake.
fn next_due_at(job: &Job, at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    const CATCH_UP: chrono::Duration = chrono::Duration::minutes(30);

    // Walk forward from the last Fire. A Job that has never fired is due at its most
    // recent occurrence, which we get by asking for the next one and stepping back a day.
    let start = match job.last_fired_at {
        Some(last) => last,
        None => {
            let next = job.schedule.next_after(at)?;
            // `next_after` is strictly after `at`, so the occurrence we want is a day
            // earlier. If that is still in the future, the Job is simply not due.
            return match next - chrono::Duration::days(1) {
                due if due <= at && at - due <= CATCH_UP => Some(due),
                _ => None,
            };
        }
    };

    let due = job.schedule.next_after(start)?;
    if due <= at && at - due <= CATCH_UP {
        Some(due)
    } else {
        None
    }
}

/// Tick the clock: fire every Job that is due, and record the ones that were missed.
///
/// Takes the database lock by path rather than a `&Connection`, so no guard is ever held
/// across the network calls inside a Fire.
pub async fn tick(
    runner: &FireRunner,
    conn: &std::sync::Mutex<rusqlite::Connection>,
    at: DateTime<Utc>,
) -> Result<(), String> {
    tick_locked(runner, conn, at).await
}

async fn tick_locked(
    runner: &FireRunner,
    conn: &std::sync::Mutex<rusqlite::Connection>,
    at: DateTime<Utc>,
) -> Result<(), String> {
    let jobs = {
        let guard = conn.lock().unwrap_or_else(|e| e.into_inner());
        store::list_jobs(&guard).map_err(|e| e.to_string())?
    };
    let mut fired_any = false;

    for job in jobs {
        if !job.enabled {
            // A disabled Job writes nothing: "skipped" would read as a missed post.
            continue;
        }
        if job.schedule.next_after(at).is_none() {
            // An unparseable schedule is a config error. Record it visibly rather than
            // silently ignoring the Job.
            let guard = conn.lock().unwrap_or_else(|e| e.into_inner());
            store::record_fire(
                &guard,
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

        // A Job is due when its most recent occurrence has passed and it has not already
        // fired for it.
        //
        // `next_after(at)` returns the next occurrence *strictly after* `at`, so it can
        // never satisfy `next <= at`. Gating on it here meant the scheduler never fired
        // anything at all. The due moment is `last_fired_at`'s successor, computed by
        // walking forward from the last Fire — or from the previous occurrence when the
        // Job has never fired.
        let due_at = next_due_at(&job, at);

        if let Some(due) = due_at {
            // The database's (job, tick) index is the real duplicate guard, so re-firing
            // within the same tick is a no-op rather than a second post.
            runner.fire(conn, &job, due).await?;
            fired_any = true;
        }
    }

    if !fired_any {
        // Still prune occasionally so the log stays bounded.
        let guard = conn.lock().unwrap_or_else(|e| e.into_inner());
        store::prune(&guard, at, DRAFT_RETENTION_DAYS, FIRE_RETENTION_DAYS)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Record Fires whose due time passed while the process was not running.
///
/// The user chose "keep the app running" over catch-up (ADR-0007), so these are marked
/// `missed` and listed in the UI, never retro-fired.
pub fn mark_missed(conn: &rusqlite::Connection, at: DateTime<Utc>) -> Result<usize, String> {
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
    use crate::domain::{ChannelRef, ContextWindow, Counts, Schedule};
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

    /// The scheduler's database handle, matching what `fire` and `tick` take.
    fn db() -> std::sync::Mutex<rusqlite::Connection> {
        std::sync::Mutex::new(store::open_in_memory().unwrap())
    }

    /// Lock the handle for a store call.
    fn c<'a>(
        conn: &'a std::sync::Mutex<rusqlite::Connection>,
    ) -> std::sync::MutexGuard<'a, rusqlite::Connection> {
        conn.lock().unwrap_or_else(|e| e.into_inner())
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
        LlmClient::new(LlmConfig::new(server.uri(), "test-model", Secret::new("k")))
    }

    #[tokio::test]
    async fn a_fire_produces_exactly_one_draft() {
        let conn = db();
        let job_id = store::insert_job(&c(&conn), &job("Day Task")).unwrap();
        let job = store::get_job(&c(&conn), job_id).unwrap();

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
        let inbox = store::list_unapproved(&c(&conn)).unwrap();
        assert_eq!(inbox.len(), 1);
        assert!(inbox[0].text.contains("jitter"));
        assert!(!inbox[0].no_signal);
    }

    #[tokio::test]
    async fn an_empty_window_produces_the_gap_marker_not_a_hedge() {
        let conn = db();
        let job_id = store::insert_job(&c(&conn), &job("Progress")).unwrap();
        let job = store::get_job(&c(&conn), job_id).unwrap();

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
        let inbox = store::list_unapproved(&c(&conn)).unwrap();
        assert_eq!(inbox[0].text, NO_SIGNAL_TEXT);
        assert!(inbox[0].no_signal);
        // No signal must not have called the model at all.
        assert_eq!(store::list_fires(&c(&conn), 10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_collect_failure_records_a_failed_fire_with_the_reason() {
        let conn = db();
        let job_id = store::insert_job(&c(&conn), &job("Day Task")).unwrap();
        let job = store::get_job(&c(&conn), job_id).unwrap();

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
        let conn = db();
        let job_id = store::insert_job(&c(&conn), &job("Day Task")).unwrap();
        let job = store::get_job(&c(&conn), job_id).unwrap();

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
        assert!(runner
            .fire(&conn, &job, at + chrono::Duration::seconds(5))
            .await
            .unwrap()
            .is_none());
        assert_eq!(store::list_fires(&c(&conn), 10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_draft_seeds_from_the_previous_cycle() {
        let conn = db();
        let job_id = store::insert_job(&c(&conn), &job("Day Task")).unwrap();
        let job = store::get_job(&c(&conn), job_id).unwrap();

        // Seed a prior Draft the "user" already wrote.
        let fire_id = store::record_fire(
            &c(&conn),
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
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
        store::insert_draft(&c(&conn), &previous).unwrap();

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
        let conn = db();
        store::insert_job(&c(&conn), &job("Day Task")).unwrap();
        let marked = mark_missed(&c(&conn), Utc::now()).unwrap();
        assert_eq!(marked, 0);
    }

    #[test]
    fn a_job_that_fired_yesterday_and_was_off_today_is_marked_missed() {
        let conn = db();
        let job_id = store::insert_job(&c(&conn), &job("Day Task")).unwrap();

        // Record a Fire from yesterday at the job's own 09:30 IST.
        let yesterday = Utc.with_ymd_and_hms(2026, 10, 1, 4, 0, 0).unwrap();
        store::record_fire(
            &c(&conn),
            job_id,
            yesterday,
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap();

        // Two days later, the app is starting up.
        let later = Utc.with_ymd_and_hms(2026, 10, 3, 12, 0, 0).unwrap();
        let marked = mark_missed(&c(&conn), later).unwrap();

        // Missed, recorded, and NOT retro-fired: no Draft was created.
        assert!(marked > 0);
        let fires = store::list_fires(&c(&conn), 50).unwrap();
        assert!(fires.iter().any(|f| f.outcome == FireOutcome::Missed));
        assert!(store::list_unapproved(&c(&conn)).unwrap().is_empty());
    }

    #[test]
    fn a_disabled_job_is_not_fired() {
        let conn = db();
        let mut disabled = job("Day Task");
        disabled.enabled = false;
        store::insert_job(&c(&conn), &disabled).unwrap();

        let at = Utc.with_ymd_and_hms(2026, 10, 2, 4, 0, 0).unwrap();
        assert_eq!(mark_missed(&c(&conn), at).unwrap(), 0);
    }

    #[tokio::test]
    async fn a_due_job_actually_fires() {
        // The regression test for the bug that made the tool non-functional: tick gated
        // on `next_after(at) <= at`, and `next_after` returns strictly after `at`, so the
        // condition was never true and no Job could ever fire.
        let conn = db();
        let job_id = store::insert_job(&*c(&conn), &job("Day Task")).unwrap();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "Day Task: some content"}}]
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

        // 04:05 UTC is 09:35 IST, five minutes after the Job's 09:30 slot.
        let at = Utc.with_ymd_and_hms(2026, 10, 2, 4, 5, 0).unwrap();
        tick(&runner, &conn, at).await.unwrap();

        let fires = store::list_fires(&*c(&conn), 10).unwrap();
        assert_eq!(
            fires.len(),
            1,
            "a Job five minutes past its slot must fire; nothing else creates a Fire"
        );
        assert_eq!(store::list_unapproved(&*c(&conn)).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_job_is_not_fired_before_its_slot() {
        let conn = db();
        store::insert_job(&*c(&conn), &job("Day Task")).unwrap();

        let server = MockServer::start().await;
        let runner = FireRunner {
            source: Arc::new(FixedSource {
                activity: Vec::new(),
                fail_with: None,
            }),
            llm: Arc::new(llm(&server)),
        };

        // 03:00 UTC is 08:30 IST, an hour before the slot.
        let at = Utc.with_ymd_and_hms(2026, 10, 2, 3, 0, 0).unwrap();
        tick(&runner, &conn, at).await.unwrap();

        assert!(
            store::list_fires(&*c(&conn), 10).unwrap().is_empty(),
            "a Job must not fire an hour before its slot"
        );
    }

    #[tokio::test]
    async fn a_job_does_not_refire_within_the_same_tick() {
        let conn = db();
        store::insert_job(&*c(&conn), &job("Day Task")).unwrap();

        let server = MockServer::start().await;
        let runner = FireRunner {
            source: Arc::new(FixedSource {
                activity: Vec::new(),
                fail_with: None,
            }),
            llm: Arc::new(llm(&server)),
        };

        // Five minutes past the slot, so both ticks are inside the catch-up window.
        let first = Utc.with_ymd_and_hms(2026, 10, 2, 4, 5, 0).unwrap();
        tick(&runner, &conn, first).await.unwrap();
        let second = first + chrono::Duration::seconds(20);
        tick(&runner, &conn, second).await.unwrap();

        // The (job, tick) index keys on the due moment, not the tick time, so the second
        // tick resolves to the same due moment and is refused.
        assert_eq!(store::list_fires(&*c(&conn), 10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_unreadable_schedule_is_recorded_as_failed_not_ignored() {
        let conn = db();
        let mut broken = job("Day Task");
        broken.schedule = Schedule::Daily {
            at: "half past nine".into(),
            tz: "Asia/Kolkata".into(),
        };
        let job_id = store::insert_job(&c(&conn), &broken).unwrap();

        let server = MockServer::start().await;
        let runner = FireRunner {
            source: Arc::new(FixedSource {
                activity: Vec::new(),
                fail_with: None,
            }),
            llm: Arc::new(llm(&server)),
        };

        tick(&runner, &conn, Utc::now()).await.unwrap();

        // A config error must be visible, not a Job that quietly never runs.
        let fires = store::list_fires(&c(&conn), 10).unwrap();
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0].job_id, job_id);
        assert_eq!(fires[0].outcome, FireOutcome::Failed);
    }
}
