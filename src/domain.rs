//! The domain vocabulary, in one place.
//!
//! Types here are the ones the ADRs name: Job, Fire, Context Window, Evidence, Draft,
//! Approve. They carry no I/O and no framework types, so both the scheduler and the HTTP
//! layer can speak them without agreeing on anything but this file.

use chrono::{DateTime, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// Where a Job's channel points.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRef {
    pub id: String,
    pub name: String,
}

/// The span of time before a Fire whose activity counts as evidence.
///
/// `Lookback` is the ordinary case. `Since` exists because a morning post is about
/// intentions and needs yesterday, not the last twelve hours — see ADR-0005 and the
/// seeding decision in ADR-0006.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextWindow {
    Lookback { hours: i64 },
    /// `at` is a wall-clock time **in the Job's timezone**, not UTC.
    Since {
        at: String,
        previous_day: bool,
        tz: String,
    },
    Explicit {
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    },
}

impl ContextWindow {
    /// Resolve the window ending at `now`.
    pub fn resolve(&self, now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
        match self {
            ContextWindow::Lookback { hours } => {
                (now - chrono::Duration::hours(*hours), now)
            }
            ContextWindow::Since {
                at,
                previous_day,
                tz,
            } => {
                // Anchored in the Job's own timezone. Resolving `at` against UTC instead
                // would shift every window by the zone offset — a 5.5-hour error for IST
                // that no test comparing against a naive expectation would catch twice.
                let zone: Option<chrono_tz::Tz> = tz.parse().ok();
                let time = at.parse::<chrono::NaiveTime>().ok();
                let day_offset: i64 = if *previous_day { -1 } else { 0 };

                let start = match (zone, time) {
                    (Some(zone), Some(time)) => {
                        let local_now = now.with_timezone(&zone);
                        // Signed day arithmetic. `Days::new` takes u64 and would drop the sign, silently
// turning "yesterday" into "today".
let day = local_now.date_naive() + chrono::Duration::days(day_offset);
                        zone.from_local_datetime(&day.and_time(time))
                            .earliest()
                            .map(|t| t.with_timezone(&Utc))
                    }
                    // Unknown zone or unparseable time: fall back to midnight UTC rather
                    // than failing the Fire, and let the Fire be visible if it matters.
                    _ => Some(
                        (now + chrono::Duration::days(day_offset))
                            .date_naive()
                            .and_hms_opt(0, 0, 0)
                            .unwrap()
                            .and_utc(),
                    ),
                };

                (start.unwrap_or(now), now)
            }
            ContextWindow::Explicit { from, to } => (*from, *to),
        }
    }

    /// The window's own description, for logs and the UI.
    pub fn label(&self) -> String {
        match self {
            ContextWindow::Lookback { hours } => format!("last {hours}h"),
            ContextWindow::Since {
                at,
                previous_day,
                ..
            } => {
                if *previous_day {
                    format!("since {at} previous day")
                } else {
                    format!("since {at}")
                }
            }
            ContextWindow::Explicit { from, to } => {
                format!("{} → {}", from.format("%d %b %H:%M"), to.format("%d %b %H:%M"))
            }
        }
    }
}

/// When a Job comes due.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    Daily {
        /// `HH:MM`, 24-hour.
        at: String,
        /// IANA timezone name, e.g. `Asia/Kolkata`. Per Job, never global.
        tz: String,
    },
}

impl Schedule {
    /// The next moment at or after `now` that this Job comes due.
    ///
    /// Returns `None` for an unparseable time or an unknown timezone: a Job that cannot
    /// compute a due time writes a `failed` Fire rather than crashing the scheduler.
    pub fn next_after(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let Schedule::Daily { at, tz } = self;
        let time = at.parse::<chrono::NaiveTime>().ok()?;
        let zone: chrono_tz::Tz = tz.parse().ok()?;

        let local_now = now.with_timezone(&zone);
        let today = local_now.date_naive();

        // Today's occurrence, or tomorrow's if it has already passed.
        let candidate_today = zone
            .from_local_datetime(&today.and_time(time))
            .earliest()?;

        let next = if candidate_today > local_now {
            candidate_today
        } else {
            let tomorrow = today + chrono::Days::new(1);
            zone.from_local_datetime(&tomorrow.and_time(time)).earliest()?
        };

        Some(next.with_timezone(&Utc))
    }
}

/// The recurring definition of one Post.
///
/// A Job is a rule. Creating or editing one sends nothing and produces no Draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub name: String,
    pub schedule: Schedule,
    pub channel: ChannelRef,
    pub context_window: ContextWindow,
    /// Instructions for writing this Post, learned from the user's own description.
    /// Empty means the Plan Role has not run yet.
    pub prompt_template: String,
    pub enabled: bool,
    pub last_fired_at: Option<DateTime<Utc>>,
}

/// The outcome of a Fire. Every terminal state, including the uneventful ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FireOutcome {
    /// A Draft was produced from complete evidence.
    Drafted,
    /// A Draft was produced, but one source failed and is named in the Evidence.
    Partial,
    /// No Draft. An error is recorded.
    Failed,
    /// The Job's time passed while the process was not running.
    Missed,
    /// A duplicate due-time within the same tick was suppressed.
    Skipped,
}

impl FireOutcome {
    /// The code shown in the spine's code column. State speaks in code.
    pub fn code(&self) -> &'static str {
        match self {
            FireOutcome::Drafted => "ACT",
            FireOutcome::Partial => "PR",
            FireOutcome::Failed => "SEV1",
            FireOutcome::Missed => "MIS",
            FireOutcome::Skipped => "SKP",
        }
    }
}

/// One occasion on which a Job came due.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fire {
    pub id: i64,
    pub job_id: i64,
    pub fired_at: DateTime<Utc>,
    pub outcome: FireOutcome,
    /// True when the Fire collected no Evidence at all. The honest gap.
    pub no_signal: bool,
    pub error: Option<String>,
}

/// Counts of what a Draft was drawn from.
///
/// Display-only, per ADR-0006: enough for the UI to say where a Draft came from without
/// keeping the source.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub slack_messages: u32,
    pub slack_channels: u32,
    pub github_comments: u32,
    pub github_repos: u32,
    /// What the API actually returned, before the token budget trimmed it.
    pub slack_fetched: u32,
    pub github_fetched: u32,
}

impl Counts {
    /// True when both integrations returned nothing. Distinguishes "nothing happened"
    /// from "we did not look", which is the whole point of the gap.
    pub fn is_empty(&self) -> bool {
        self.slack_messages == 0 && self.github_comments == 0
    }

    /// How much of what was fetched survived the trim.
    pub fn is_truncated(&self) -> bool {
        self.slack_messages < self.slack_fetched || self.github_comments < self.github_fetched
    }
}

/// A proposed Post, awaiting a human.
///
/// Inert by construction: nothing here can reach Slack. Only `approve_draft` can.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    pub id: i64,
    pub job_id: i64,
    pub fire_id: i64,
    pub text: String,
    pub window_from: DateTime<Utc>,
    pub window_to: DateTime<Utc>,
    pub counts: Counts,
    /// True when this Draft drew from nothing. Carries NO SIGNAL rather than filler.
    pub no_signal: bool,
    /// True when one source failed. The Draft says which.
    pub partial: bool,
    /// The Draft this one was seeded from. Resolves through discarded ancestors.
    pub parent_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub edited: bool,
    /// True once a human approved it. There is no path to Slack without this.
    pub approved: bool,
    pub approved_at: Option<DateTime<Utc>>,
    /// Hidden from the Inbox but retained, so descendants keep their lineage.
    pub discarded: bool,
}

/// UTC now, named so tests read clearly.
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// Local wall-clock rendering, for display only. Never for scheduling.
pub fn local_string(t: DateTime<Utc>, fmt: &str) -> String {
    t.with_timezone(&Local).format(fmt).to_string()
}

/// A monotonic-ish marker used to suppress duplicate due-times within one tick.
pub fn tick_key(now: DateTime<Utc>) -> i64 {
    now.timestamp() / 20
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn daily_schedule_returns_today_when_still_ahead() {
        let schedule = Schedule::Daily {
            at: "14:30".into(),
            tz: "Asia/Kolkata".into(),
        };
        // 08:00 UTC is 13:30 IST, so 14:30 IST is an hour out.
        let now = utc(2026, 10, 2, 8, 0);
        let next = schedule.next_after(now).unwrap();
        assert_eq!(next, utc(2026, 10, 2, 9, 0));
    }

    #[test]
    fn daily_schedule_rolls_to_tomorrow_when_passed() {
        let schedule = Schedule::Daily {
            at: "09:30".into(),
            tz: "Asia/Kolkata".into(),
        };
        // 10:00 UTC is 15:30 IST — 09:30 IST has gone.
        let next = schedule.next_after(utc(2026, 10, 2, 10, 0)).unwrap();
        assert_eq!(next, utc(2026, 10, 3, 4, 0));
    }

    #[test]
    fn schedule_is_per_job_timezone_not_machine_timezone() {
        let ist = Schedule::Daily {
            at: "09:30".into(),
            tz: "Asia/Kolkata".into(),
        };
        let pst = Schedule::Daily {
            at: "09:30".into(),
            tz: "America/Los_Angeles".into(),
        };
        let now = utc(2026, 10, 2, 12, 0);
        // Same wall time, two zones: the due moments differ by a full working day.
        assert_ne!(ist.next_after(now), pst.next_after(now));
    }

    #[test]
    fn unparseable_schedule_yields_none_rather_than_panicking() {
        let schedule = Schedule::Daily {
            at: "half past nine".into(),
            tz: "Asia/Kolkata".into(),
        };
        assert_eq!(schedule.next_after(utc(2026, 10, 2, 8, 0)), None);
    }

    #[test]
    fn unknown_timezone_yields_none() {
        let schedule = Schedule::Daily {
            at: "09:30".into(),
            tz: "Mars/Olympus".into(),
        };
        assert_eq!(schedule.next_after(utc(2026, 10, 2, 8, 0)), None);
    }

    #[test]
    fn lookback_window_spans_the_stated_hours() {
        let now = utc(2026, 10, 2, 14, 30);
        let (from, to) = ContextWindow::Lookback { hours: 5 }.resolve(now);
        assert_eq!(to, now);
        assert_eq!(from, utc(2026, 10, 2, 9, 30));
    }

    #[test]
    fn since_previous_day_anchors_to_the_jobs_timezone() {
        let now = utc(2026, 10, 2, 4, 0);
        let (from, to) = ContextWindow::Since {
            at: "18:30".into(),
            previous_day: true,
            tz: "Asia/Kolkata".into(),
        }
        .resolve(now);
        assert_eq!(to, now);
        // 18:30 IST on 1 Oct is 13:00 UTC, not 18:30 UTC. The zone offset is the point.
        assert_eq!(from, utc(2026, 10, 1, 13, 0));
    }

    #[test]
    fn since_window_shifts_with_the_timezone() {
        let now = utc(2026, 10, 2, 4, 0);
        let ist = ContextWindow::Since {
            at: "18:30".into(),
            previous_day: true,
            tz: "Asia/Kolkata".into(),
        };
        let pst = ContextWindow::Since {
            at: "18:30".into(),
            previous_day: true,
            tz: "America/Los_Angeles".into(),
        };
        assert_ne!(ist.resolve(now).0, pst.resolve(now).0);
    }

    #[test]
    fn unparseable_since_falls_back_to_midnight_utc() {
        let now = utc(2026, 10, 2, 4, 0);
        let (from, _) = ContextWindow::Since {
            at: "not a time".into(),
            previous_day: true,
            tz: "Asia/Kolkata".into(),
        }
        .resolve(now);
        assert_eq!(from, utc(2026, 10, 1, 0, 0));
    }

    #[test]
    fn empty_counts_report_no_signal() {
        let counts = Counts {
            slack_fetched: 0,
            github_fetched: 0,
            ..Counts::default()
        };
        assert!(counts.is_empty());
    }

    #[test]
    fn fetched_without_surviving_is_truncated_not_empty() {
        // 200 fetched, 15 survived the budget. Not empty, but visibly cut.
        let counts = Counts {
            slack_messages: 15,
            slack_fetched: 200,
            ..Counts::default()
        };
        assert!(!counts.is_empty());
        assert!(counts.is_truncated());
    }

    #[test]
    fn outcome_codes_are_stable() {
        assert_eq!(FireOutcome::Drafted.code(), "ACT");
        assert_eq!(FireOutcome::Failed.code(), "SEV1");
        assert_eq!(FireOutcome::Missed.code(), "MIS");
    }

    #[test]
    fn tick_key_collapses_a_twenty_second_window() {
        let base = utc(2026, 10, 2, 12, 0);
        assert_eq!(tick_key(base), tick_key(base + chrono::Duration::seconds(19)));
        assert_ne!(tick_key(base + chrono::Duration::seconds(19)), tick_key(base + chrono::Duration::seconds(20)));
    }
}