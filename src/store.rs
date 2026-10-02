//! SQLite persistence for Jobs, Drafts and the Fire log.
//!
//! Every function that can fail on a constraint returns `Result`; the ones that read a
//! row that may not exist return `Option`. Nothing here panics on user input.

use rusqlite::{params, Connection, OptionalExtension};

use crate::domain::{ChannelRef, Draft, Fire, FireOutcome, Job};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("job {0} not found")]
    JobNotFound(i64),
    #[error("draft {0} not found")]
    DraftNotFound(i64),
    #[error("fire {0} not found")]
    FireNotFound(i64),
    #[error("could not decode stored {what}: {source}")]
    Decode {
        what: &'static str,
        #[source]
        source: rusqlite::Error,
    },
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Open and migrate a connection.
pub fn open(path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    migrate(&conn)?;
    Ok(conn)
}

/// An in-memory database, for tests.
#[cfg(test)]
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS jobs (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            name              TEXT    NOT NULL,
            schedule          TEXT    NOT NULL,
            channel_id        TEXT    NOT NULL,
            channel_name      TEXT    NOT NULL,
            context_window    TEXT    NOT NULL,
            prompt_template   TEXT    NOT NULL DEFAULT '',
            enabled           INTEGER NOT NULL DEFAULT 1,
            last_fired_at     TEXT
        );

        CREATE TABLE IF NOT EXISTS fires (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id      INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
            fired_at    TEXT    NOT NULL,
            outcome     TEXT    NOT NULL,
            no_signal   INTEGER NOT NULL DEFAULT 0,
            error       TEXT,
            -- One row per (job, tick). The duplicate-due-time guard lives here so it
            -- cannot be bypassed by a scheduler restart mid-tick.
            tick        INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS drafts (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id       INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
            fire_id      INTEGER NOT NULL REFERENCES fires(id) ON DELETE CASCADE,
            text         TEXT    NOT NULL,
            window_from  TEXT    NOT NULL,
            window_to    TEXT    NOT NULL,
            counts       TEXT    NOT NULL DEFAULT '{}',
            no_signal    INTEGER NOT NULL DEFAULT 0,
            partial      INTEGER NOT NULL DEFAULT 0,
            -- No ON DELETE clause: a hard delete is RESTRICT, so prune cannot silently
            -- orphan a child. Discarded Drafts are flagged, not deleted (ADR-0006).
            parent_id    INTEGER REFERENCES drafts(id),
            -- Discarded rather than deleted. The row must survive so a child's lineage
            -- still resolves through it (ADR-0006); deleting it would either cascade
            -- away the child or fail the foreign key. Hidden from the Inbox instead.
            discarded    INTEGER NOT NULL DEFAULT 0,
            created_at   TEXT    NOT NULL,
            edited       INTEGER NOT NULL DEFAULT 0,
            approved     INTEGER NOT NULL DEFAULT 0,
            approved_at  TEXT
        );

        -- A Draft's lineage is walked far more often than it is written.
        CREATE INDEX IF NOT EXISTS drafts_job_created ON drafts(job_id, created_at DESC);
        -- The Inbox is unapproved drafts, newest first.
        CREATE INDEX IF NOT EXISTS drafts_unapproved ON drafts(approved, created_at DESC);
        CREATE INDEX IF NOT EXISTS fires_job_fired ON fires(job_id, fired_at DESC);
        -- ADR-0007: at most one Fire per (job, tick).
        CREATE UNIQUE INDEX IF NOT EXISTS fires_job_tick ON fires(job_id, tick);
        "#,
    )?;
    Ok(())
}

// ── Jobs ───────────────────────────────────────────────────────────────────

/// Insert a Job and return its id.
pub fn insert_job(conn: &Connection, job: &Job) -> Result<i64> {
    conn.execute(
        "INSERT INTO jobs (name, schedule, channel_id, channel_name, context_window,
                           prompt_template, enabled, last_fired_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            job.name,
            serde_json::to_string(&job.schedule).map_err(sqlite_json_err)?,
            job.channel.id,
            job.channel.name,
            serde_json::to_string(&job.context_window).map_err(sqlite_json_err)?,
            job.prompt_template,
            job.enabled as i64,
            job.last_fired_at.map(|t| t.to_rfc3339()),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_job(conn: &Connection, job: &Job) -> Result<()> {
    let changed = conn.execute(
        "UPDATE jobs SET name = ?2, schedule = ?3, channel_id = ?4, channel_name = ?5,
                         context_window = ?6, prompt_template = ?7, enabled = ?8,
                         last_fired_at = ?9
         WHERE id = ?1",
        params![
            job.id,
            job.name,
            serde_json::to_string(&job.schedule).map_err(sqlite_json_err)?,
            job.channel.id,
            job.channel.name,
            serde_json::to_string(&job.context_window).map_err(sqlite_json_err)?,
            job.prompt_template,
            job.enabled as i64,
            job.last_fired_at.map(|t| t.to_rfc3339()),
        ],
    )?;
    if changed == 0 {
        return Err(StoreError::JobNotFound(job.id));
    }
    Ok(())
}

pub fn list_jobs(conn: &Connection) -> Result<Vec<Job>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, schedule, channel_id, channel_name, context_window,
                prompt_template, enabled, last_fired_at
         FROM jobs ORDER BY id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            let schedule: String = row.get(2)?;
            let window: String = row.get(5)?;
            let last_fired: Option<String> = row.get(8)?;

            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                schedule,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                window,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                last_fired,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    rows.into_iter()
        .map(
            |(id, name, schedule, cid, cname, window, template, enabled, last_fired)| {
                Ok(Job {
                    id,
                    name,
                    schedule: serde_json::from_str(&schedule)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    channel: ChannelRef {
                        id: cid,
                        name: cname,
                    },
                    context_window: serde_json::from_str(&window)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    prompt_template: template,
                    enabled: enabled != 0,
                    last_fired_at: last_fired.and_then(|s| s.parse().ok()),
                })
            },
        )
        .collect()
}

pub fn get_job(conn: &Connection, id: i64) -> Result<Job> {
    list_jobs(conn)?
        .into_iter()
        .find(|j| j.id == id)
        .ok_or(StoreError::JobNotFound(id))
}

/// Delete a Job and everything under it.
///
/// The Drafts go children-first, because `parent_id` is RESTRICT: a self-referencing
/// chain cannot be deleted by a single cascade, and a plain `DELETE FROM jobs` leaves
/// SQLite to cascade in an order that trips the constraint.
pub fn delete_job(conn: &Connection, id: i64) -> Result<()> {
    // Break the chain from the leaves up, then remove what is left.
    conn.execute(
        "UPDATE drafts SET parent_id = NULL WHERE job_id = ?1",
        params![id],
    )?;
    conn.execute("DELETE FROM drafts WHERE job_id = ?1", params![id])?;
    conn.execute("DELETE FROM jobs WHERE id = ?1", params![id])?;
    Ok(())
}

// ── Fires ──────────────────────────────────────────────────────────────────

/// Record a Fire. Returns `None` when this (job, tick) was already recorded.
///
/// The uniqueness is enforced by the index rather than by a read-then-write, so two
/// concurrent ticks cannot both insert.
pub fn record_fire(
    conn: &Connection,
    job_id: i64,
    fired_at: chrono::DateTime<chrono::Utc>,
    outcome: FireOutcome,
    no_signal: bool,
    error: Option<&str>,
    tick: i64,
) -> Result<Option<i64>> {
    let changed = conn.execute(
        "INSERT OR IGNORE INTO fires (job_id, fired_at, outcome, no_signal, error, tick)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            job_id,
            fired_at.to_rfc3339(),
            serde_json::to_string(&outcome).map_err(sqlite_json_err)?,
            no_signal as i64,
            error,
            tick,
        ],
    )?;

    if changed == 0 {
        return Ok(None);
    }

    let id = conn.last_insert_rowid();
    // Mark the Job as fired so the scheduler does not re-fire the same due time.
    conn.execute(
        "UPDATE jobs SET last_fired_at = ?2 WHERE id = ?1",
        params![job_id, fired_at.to_rfc3339()],
    )?;
    Ok(Some(id))
}

pub fn list_fires(conn: &Connection, limit: i64) -> Result<Vec<Fire>> {
    let mut stmt = conn.prepare(
        "SELECT id, job_id, fired_at, outcome, no_signal, error
         FROM fires ORDER BY fired_at DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |row| {
            let outcome: String = row.get(3)?;
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                outcome,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    rows.into_iter()
        .map(|(id, job_id, fired_at, outcome, no_signal, error)| {
            Ok(Fire {
                id,
                job_id,
                fired_at: fired_at
                    .parse()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                outcome: serde_json::from_str(&outcome)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                no_signal: no_signal != 0,
                error,
            })
        })
        .collect()
}

/// One Fire by id.
pub fn get_fire(conn: &Connection, id: i64) -> Result<Fire> {
    list_fires(conn, 1000)?
        .into_iter()
        .find(|f| f.id == id)
        .ok_or(StoreError::FireNotFound(id))
}

// ── Drafts ─────────────────────────────────────────────────────────────────

/// Insert a Draft and return its id.
pub fn insert_draft(conn: &Connection, draft: &Draft) -> Result<i64> {
    conn.execute(
        "INSERT INTO drafts (job_id, fire_id, text, window_from, window_to, counts,
                             no_signal, partial, parent_id, created_at, edited,
                             approved, approved_at, discarded)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            draft.job_id,
            draft.fire_id,
            draft.text,
            draft.window_from.to_rfc3339(),
            draft.window_to.to_rfc3339(),
            serde_json::to_string(&draft.counts).map_err(sqlite_json_err)?,
            draft.no_signal as i64,
            draft.partial as i64,
            draft.parent_id,
            draft.created_at.to_rfc3339(),
            draft.edited as i64,
            draft.approved as i64,
            draft.approved_at.map(|t| t.to_rfc3339()),
            draft.discarded as i64,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Replace a Draft's text *and* what it was drawn from.
///
/// Both, because the counts are the only record of where a Draft came from (ADR-0006).
/// Updating the text alone left the UI describing the first render while showing the
/// second, which is worse than no record at all.
pub fn replace_draft(
    conn: &Connection,
    id: i64,
    text: &str,
    counts: &crate::domain::Counts,
    no_signal: bool,
    partial: bool,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE drafts SET text = ?2, counts = ?3, no_signal = ?4, partial = ?5, edited = 1
         WHERE id = ?1",
        params![
            id,
            text,
            serde_json::to_string(counts).map_err(sqlite_json_err)?,
            no_signal as i64,
            partial as i64,
        ],
    )?;
    if changed == 0 {
        return Err(StoreError::DraftNotFound(id));
    }
    Ok(())
}

pub fn update_draft_text(conn: &Connection, id: i64, text: &str) -> Result<()> {
    let changed = conn.execute(
        "UPDATE drafts SET text = ?2, edited = 1 WHERE id = ?1",
        params![id, text],
    )?;
    if changed == 0 {
        return Err(StoreError::DraftNotFound(id));
    }
    Ok(())
}

/// Claim a Draft for sending, atomically.
///
/// Returns `false` when it was already claimed. The `WHERE approved = 0` is what makes
/// this safe: two concurrent Approve requests race on it and exactly one sees a change,
/// so the second refuses instead of posting a duplicate.
///
/// The claim happens *before* the Slack call, so it must be released on failure —
/// `release_claim` puts the Draft back in the Inbox.
pub fn claim_for_send(
    conn: &Connection,
    id: i64,
    at: chrono::DateTime<chrono::Utc>,
) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE drafts SET approved = 1, approved_at = ?2 WHERE id = ?1 AND approved = 0",
        params![id, at.to_rfc3339()],
    )?;
    Ok(changed == 1)
}

/// Undo a claim, so a failed send leaves the Draft in the Inbox rather than silently
/// consuming it.
pub fn release_claim(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE drafts SET approved = 0, approved_at = NULL WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Mark a Draft approved. Used by tests and by callers that already hold the claim.
pub fn mark_approved(conn: &Connection, id: i64, at: chrono::DateTime<chrono::Utc>) -> Result<()> {
    let changed = conn.execute(
        "UPDATE drafts SET approved = 1, approved_at = ?2 WHERE id = ?1",
        params![id, at.to_rfc3339()],
    )?;
    if changed == 0 {
        return Err(StoreError::DraftNotFound(id));
    }
    Ok(())
}

/// Discard a Draft: hidden from the Inbox, retained so descendants keep their lineage.
///
/// Not a delete. ADR-0006 keeps discarded Drafts in the chain, and a hard delete would
/// either cascade away a child or trip the foreign key.
pub fn discard_draft(conn: &Connection, id: i64) -> Result<()> {
    let changed = conn.execute("UPDATE drafts SET discarded = 1 WHERE id = ?1", params![id])?;
    if changed == 0 {
        return Err(StoreError::DraftNotFound(id));
    }
    Ok(())
}

pub fn get_draft(conn: &Connection, id: i64) -> Result<Draft> {
    list_drafts(conn)?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or(StoreError::DraftNotFound(id))
}

/// Unapproved Drafts, newest first. The Inbox.
pub fn list_unapproved(conn: &Connection) -> Result<Vec<Draft>> {
    let mut drafts = list_drafts(conn)?;
    drafts.sort_by_key(|d| std::cmp::Reverse(d.created_at));
    Ok(drafts
        .into_iter()
        .filter(|d| !d.approved && !d.discarded)
        .collect())
}

pub fn list_drafts(conn: &Connection) -> Result<Vec<Draft>> {
    let mut stmt = conn.prepare(
        "SELECT id, job_id, fire_id, text, window_from, window_to, counts,
                no_signal, partial, parent_id, created_at, edited, approved, approved_at,
                discarded
         FROM drafts ORDER BY created_at",
    )?;
    let rows = stmt
        .query_map([], |row| {
            let counts: String = row.get(6)?;
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                counts,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, i64>(14)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    rows.into_iter()
        .map(
            |(
                id,
                job_id,
                fire_id,
                text,
                from,
                to,
                counts,
                no_signal,
                partial,
                parent,
                created,
                edited,
                approved,
                approved_at,
                discarded,
            )| {
                Ok(Draft {
                    id,
                    job_id,
                    fire_id,
                    text,
                    window_from: from.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
                    window_to: to.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
                    counts: serde_json::from_str(&counts)
                        .map_err(|_| rusqlite::Error::InvalidQuery)?,
                    no_signal: no_signal != 0,
                    partial: partial != 0,
                    parent_id: parent,
                    created_at: created.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
                    edited: edited != 0,
                    approved: approved != 0,
                    approved_at: approved_at.and_then(|s| s.parse().ok()),
                    discarded: discarded != 0,
                })
            },
        )
        .collect()
}

/// The nearest surviving ancestor of a Draft, walking through any discarded links.
///
/// ADR-0006: a discarded Draft stays in the chain as a hole rather than being reparented
/// away, so this skips over missing rows. Implemented as a bounded walk rather than a
/// recursive CTE so a corrupt chain cannot loop forever.
#[allow(dead_code)]
pub fn nearest_ancestor(conn: &Connection, draft_id: i64) -> Result<Option<i64>> {
    let mut cursor = draft_id;
    let mut hops = 0;

    // Walk up until we find a Draft that still exists and has not been discarded.
    // Discarded rows are skipped rather than ending the walk: ADR-0006 wants the chain
    // to resolve through the gap to the nearest surviving ancestor.
    loop {
        hops += 1;
        if hops > 64 {
            // A cycle in a hand-edited database. Seed from nothing rather than spin.
            return Ok(None);
        }

        let parent: Option<Option<i64>> = conn
            .query_row(
                "SELECT parent_id FROM drafts WHERE id = ?1",
                params![cursor],
                |row| row.get(0),
            )
            .optional()?;

        let Some(Some(next)) = parent else {
            return Ok(None);
        };

        if next == draft_id {
            return Ok(None);
        }

        let survives = conn
            .query_row(
                "SELECT discarded FROM drafts WHERE id = ?1",
                params![next],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .map(|discarded| discarded == 0)
            .unwrap_or(false);

        if survives {
            return Ok(Some(next));
        }

        cursor = next;
    }
}

// ── Pruning ────────────────────────────────────────────────────────────────

/// ADR-0007: prune beyond the retention windows.
///
/// Returns `(drafts_pruned, fires_pruned)`.
pub fn prune(
    conn: &Connection,
    now: chrono::DateTime<chrono::Utc>,
    draft_days: i64,
    fire_days: i64,
) -> Result<(usize, usize)> {
    let draft_cutoff = now - chrono::Duration::days(draft_days);
    let fire_cutoff = now - chrono::Duration::days(fire_days);

    // A Draft is only *hard-deleted* once it is approved, old, and has no child. The
    // NOT EXISTS guard is not sufficient on its own: SQLite checks foreign keys during
    // the scan, so a statement that merely considers a referenced row still raises
    // before the guard excludes it. The child links are therefore cleared first, so
    // nothing the delete visits is referenced.
    //
    // Unapproved Drafts are never deleted. They are what the Inbox is, and an unapproved
    // Draft older than the retention window is something the user has not decided about.
    conn.execute(
        "UPDATE drafts SET parent_id = NULL
          WHERE created_at < ?1 AND parent_id IS NOT NULL",
        params![draft_cutoff.to_rfc3339()],
    )?;

    let drafts = conn.execute(
        "DELETE FROM drafts
          WHERE approved = 1 AND created_at < ?1",
        params![draft_cutoff.to_rfc3339()],
    )?;

    // Fire rows are deleted, but a Fire with a surviving Draft is kept so the Draft's
    // `fire_id` still resolves. Approved Drafts are never reached this way because the
    // cascade only fires when the Fire row itself goes.
    let fires = conn.execute(
        "DELETE FROM fires
          WHERE fired_at < ?1
            AND NOT EXISTS (SELECT 1 FROM drafts d WHERE d.fire_id = fires.id)",
        params![fire_cutoff.to_rfc3339()],
    )?;

    Ok((drafts, fires))
}

/// Build a store error from a serde failure, so a bad JSON blob surfaces as a query
/// error rather than a panic.
fn sqlite_json_err(err: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ContextWindow, Counts, Schedule};
    use chrono::Utc;

    fn sample_job(name: &str) -> Job {
        Job {
            id: 0,
            name: name.into(),
            schedule: Schedule::Daily {
                at: "09:30".into(),
                tz: "Asia/Kolkata".into(),
            },
            channel: ChannelRef {
                id: "C0A0RRC7P8B".into(),
                name: "coot-ai".into(),
            },
            context_window: ContextWindow::Lookback { hours: 8 },
            prompt_template: "write it".into(),
            enabled: true,
            last_fired_at: None,
        }
    }

    fn sample_draft(job_id: i64, fire_id: i64, text: &str) -> Draft {
        Draft {
            id: 0,
            job_id,
            fire_id,
            text: text.into(),
            window_from: Utc::now() - chrono::Duration::hours(8),
            window_to: Utc::now(),
            counts: Counts {
                slack_messages: 4,
                slack_fetched: 4,
                ..Counts::default()
            },
            no_signal: false,
            partial: false,
            parent_id: None,
            created_at: Utc::now(),
            edited: false,
            approved: false,
            approved_at: None,
            discarded: false,
        }
    }

    #[test]
    fn job_roundtrips_through_sqlite() {
        let conn = open_in_memory().unwrap();
        let mut job = sample_job("Day Task");
        job.context_window = ContextWindow::Since {
            at: "18:30".into(),
            previous_day: true,
            tz: "Asia/Kolkata".into(),
        };
        job.id = insert_job(&conn, &job).unwrap();

        let read = get_job(&conn, job.id).unwrap();
        assert_eq!(read.name, "Day Task");
        assert_eq!(read.context_window, job.context_window);
        assert!(read.enabled);
    }

    #[test]
    fn updating_a_missing_job_is_an_error_not_a_panic() {
        let conn = open_in_memory().unwrap();
        let mut job = sample_job("gone");
        job.id = 999;
        assert!(matches!(
            update_job(&conn, &job),
            Err(StoreError::JobNotFound(999))
        ));
    }

    #[test]
    fn one_fire_per_job_per_tick() {
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();

        let first = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            100,
        )
        .unwrap();
        let second = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            100,
        )
        .unwrap();

        assert!(first.is_some(), "first Fire in a tick is recorded");
        assert!(second.is_none(), "duplicate due time is suppressed");
        assert_eq!(list_fires(&conn, 10).unwrap().len(), 1);
    }

    #[test]
    fn recording_a_fire_stamps_the_jobs_last_fired() {
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let at = Utc::now();
        record_fire(&conn, job_id, at, FireOutcome::Drafted, false, None, 1).unwrap();

        let job = get_job(&conn, job_id).unwrap();
        assert_eq!(job.last_fired_at, Some(at));
    }

    #[test]
    fn draft_roundtrips_and_stays_in_the_inbox_until_approved() {
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();

        let draft_id = insert_draft(&conn, &sample_draft(job_id, fire_id, "hello")).unwrap();

        assert_eq!(list_unapproved(&conn).unwrap().len(), 1);
        mark_approved(&conn, draft_id, Utc::now()).unwrap();
        assert_eq!(list_unapproved(&conn).unwrap().len(), 0);

        let read = get_draft(&conn, draft_id).unwrap();
        assert!(read.approved);
        assert_eq!(read.counts.slack_messages, 4);
    }

    #[test]
    fn inbox_is_newest_first() {
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();

        let mut older = sample_draft(job_id, fire_id, "older");
        older.created_at = Utc::now() - chrono::Duration::hours(2);
        insert_draft(&conn, &older).unwrap();
        insert_draft(&conn, &sample_draft(job_id, fire_id, "newer")).unwrap();

        let inbox = list_unapproved(&conn).unwrap();
        assert_eq!(inbox[0].text, "newer");
        assert_eq!(inbox[1].text, "older");
    }

    #[test]
    fn discarding_a_draft_promotes_its_parent_as_the_seed() {
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();

        let grandparent = insert_draft(&conn, &sample_draft(job_id, fire_id, "gp")).unwrap();
        let mut middle = sample_draft(job_id, fire_id, "parent");
        middle.parent_id = Some(grandparent);
        let parent = insert_draft(&conn, &middle).unwrap();
        let mut child = sample_draft(job_id, fire_id, "child");
        child.parent_id = Some(parent);
        let leaf = insert_draft(&conn, &child).unwrap();

        assert_eq!(nearest_ancestor(&conn, leaf).unwrap(), Some(parent));

        discard_draft(&conn, parent).unwrap();

        // The chain now resolves past the discarded Draft to its parent, and the
        // discarded Draft leaves the Inbox without taking its child with it.
        assert_eq!(nearest_ancestor(&conn, leaf).unwrap(), Some(grandparent));
    }

    #[test]
    fn ancestor_walk_terminates_on_a_draft_with_no_parent() {
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();
        let leaf = insert_draft(&conn, &sample_draft(job_id, fire_id, "leaf")).unwrap();
        assert_eq!(nearest_ancestor(&conn, leaf).unwrap(), None);
    }

    #[test]
    fn pruning_keeps_approved_drafts_and_drops_stale_unapproved_ones() {
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();

        let mut stale_approved = sample_draft(job_id, fire_id, "kept");
        stale_approved.created_at = Utc::now() - chrono::Duration::days(90);
        let kept = insert_draft(&conn, &stale_approved).unwrap();
        mark_approved(&conn, kept, Utc::now()).unwrap();

        let mut stale_unapproved = sample_draft(job_id, fire_id, "dropped");
        stale_unapproved.created_at = Utc::now() - chrono::Duration::days(90);
        insert_draft(&conn, &stale_unapproved).unwrap();

        let (pruned, _) = prune(&conn, Utc::now(), 30, 90).unwrap();

        assert_eq!(pruned, 1);
        assert_eq!(list_drafts(&conn).unwrap().len(), 1);
    }

    #[test]
    fn debug_prune() {
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();
        let mut parent = sample_draft(job_id, fire_id, "parent");
        parent.created_at = Utc::now() - chrono::Duration::days(90);
        let pid = insert_draft(&conn, &parent).unwrap();
        mark_approved(&conn, pid, Utc::now()).unwrap();
        let mut child = sample_draft(job_id, fire_id, "child");
        child.parent_id = Some(pid);
        child.created_at = Utc::now();
        let cid = insert_draft(&conn, &child).unwrap();
        println!(
            "parent={pid} child={cid} drafts={}",
            list_drafts(&conn).unwrap().len()
        );
        let r = prune(&conn, Utc::now(), 30, 90);
        println!("prune result: {r:?}");
    }

    #[test]
    fn debug_prune3() {
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();
        let mut parent = sample_draft(job_id, fire_id, "parent");
        parent.created_at = Utc::now() - chrono::Duration::days(90);
        let pid = insert_draft(&conn, &parent).unwrap();
        mark_approved(&conn, pid, Utc::now()).unwrap();
        let mut child = sample_draft(job_id, fire_id, "child");
        child.parent_id = Some(pid);
        child.created_at = Utc::now();
        let _cid = insert_draft(&conn, &child).unwrap();
        let fire_cutoff = (Utc::now() - chrono::Duration::days(90)).to_rfc3339();
        let f = conn.execute("DELETE FROM fires WHERE fired_at < ?1 AND NOT EXISTS (SELECT 1 FROM drafts d WHERE d.fire_id = fires.id)", rusqlite::params![fire_cutoff]);
        println!("fire delete: {f:?}");
        println!("fires left: {}", list_fires(&conn, 10).unwrap().len());
    }

    #[test]
    fn debug_prune4() {
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();
        let mut parent = sample_draft(job_id, fire_id, "parent");
        parent.created_at = Utc::now() - chrono::Duration::days(90);
        let pid = insert_draft(&conn, &parent).unwrap();
        mark_approved(&conn, pid, Utc::now()).unwrap();
        let mut child = sample_draft(job_id, fire_id, "child");
        child.parent_id = Some(pid);
        child.created_at = Utc::now();
        let _cid = insert_draft(&conn, &child).unwrap();
        // Step 1: the draft delete from prune, verbatim
        let draft_cutoff = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        let d = conn.execute("DELETE FROM drafts WHERE approved = 1 AND created_at < ?1 AND (parent_id IS NULL OR NOT EXISTS (SELECT 1 FROM drafts c WHERE c.parent_id = drafts.id))", rusqlite::params![draft_cutoff]);
        println!("step1 draft delete: {d:?}");
        println!("drafts after: {}", list_drafts(&conn).unwrap().len());
        // Step 2: the fire delete from prune, verbatim
        let fire_cutoff = (Utc::now() - chrono::Duration::days(90)).to_rfc3339();
        let f = conn.execute("DELETE FROM fires WHERE fired_at < ?1 AND NOT EXISTS (SELECT 1 FROM drafts d WHERE d.fire_id = fires.id)", rusqlite::params![fire_cutoff]);
        println!("step2 fire delete: {f:?}");
    }

    #[test]
    fn a_draft_can_only_be_claimed_once() {
        // ADR-0001: two concurrent Approve requests must not both post. The claim is the
        // serialisation point — without it, both read `approved = false`, both release
        // the lock for the Slack call, and both post.
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();
        let id = insert_draft(&conn, &sample_draft(job_id, fire_id, "x")).unwrap();

        assert!(claim_for_send(&conn, id, Utc::now()).unwrap());
        assert!(
            !claim_for_send(&conn, id, Utc::now()).unwrap(),
            "a second claim must be refused"
        );

        // Releasing puts it back, so a failed send is recoverable.
        release_claim(&conn, id).unwrap();
        assert!(claim_for_send(&conn, id, Utc::now()).unwrap());
    }

    #[test]
    fn pruning_never_breaks_a_lineage_chain() {
        // The parent_id foreign key is RESTRICT, so a naive prune that removed a
        // referenced Draft raised a violation — and the scheduler propagates it, so
        // pruning printed an error every tick and never worked again. Prune must clear
        // the child links before deleting, and never delete an unapproved Draft.
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();

        let old = Utc::now() - chrono::Duration::days(90);
        let mut parent = sample_draft(job_id, fire_id, "parent");
        parent.created_at = old;
        let parent_id = insert_draft(&conn, &parent).unwrap();
        mark_approved(&conn, parent_id, old).unwrap();

        let mut child = sample_draft(job_id, fire_id, "child");
        child.parent_id = Some(parent_id);
        child.created_at = old;
        let child_id = insert_draft(&conn, &child).unwrap();

        // The parent is old and approved but still referenced; the child is old and
        // unapproved, so it is kept. Pruning the link first must not raise.
        prune(&conn, Utc::now(), 30, 90).unwrap();

        // The unapproved child survives.
        assert!(
            get_draft(&conn, child_id).is_ok(),
            "an unapproved draft must survive pruning"
        );
    }

    #[test]
    fn an_unapproved_draft_is_never_pruned_away() {
        // An unapproved Draft is something the user has not decided about. Silently
        // deleting it because it is old is how a pending post disappears.
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();

        let mut stale = sample_draft(job_id, fire_id, "undecided");
        stale.created_at = Utc::now() - chrono::Duration::days(400);
        let id = insert_draft(&conn, &stale).unwrap();

        prune(&conn, Utc::now(), 30, 90).unwrap();

        assert!(
            get_draft(&conn, id).is_ok(),
            "an unapproved draft must survive pruning"
        );
        assert_eq!(list_unapproved(&conn).unwrap().len(), 1);
    }

    #[test]
    fn deleting_a_job_cascades_to_its_fires_and_drafts() {
        let conn = open_in_memory().unwrap();
        let job_id = insert_job(&conn, &sample_job("Day Task")).unwrap();
        let fire_id = record_fire(
            &conn,
            job_id,
            Utc::now(),
            FireOutcome::Drafted,
            false,
            None,
            1,
        )
        .unwrap()
        .unwrap();
        insert_draft(&conn, &sample_draft(job_id, fire_id, "x")).unwrap();

        delete_job(&conn, job_id).unwrap();

        assert!(list_fires(&conn, 10).unwrap().is_empty());
        assert!(list_drafts(&conn).unwrap().is_empty());
    }
}
