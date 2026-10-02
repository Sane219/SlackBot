#!/usr/bin/env bash
# Fill a throwaway database with Jobs, Fires and Drafts so the UI can be looked at in
# the states that matter. Never run against a real database.
#
#   cargo run                                   # once, to create the schema
#   scripts/seed-demo-db.sh "$SLACKBOT_DATA_DIR/slackbot.db"
#
# The server can keep running while this writes: every read is a fresh query.
#
# Note the JSON in `fires.outcome` and `jobs.schedule`. Those columns hold
# serde-encoded values, so a bare `drafted` or `daily` is not valid and every read of the
# row fails. Writing one is how this seed spent an afternoon looking for a database bug.
set -euo pipefail

db="${1:?usage: seed-demo-db.sh <path-to.db>}"
[ -f "$db" ] || { echo "no database at $db" >&2; exit 1; }

# Refuse to run twice. Appending to an already-seeded database leaves drafts pointing at
# fire ids that were never inserted, which looks exactly like a data bug.
existing=$(sqlite3 "$db" "SELECT count(*) FROM jobs;" 2>/dev/null || echo 0)
if [ "$existing" != "0" ]; then
  echo "$db already has $existing job(s). Delete it and restart the server first." >&2
  exit 1
fi

now=$(date -u +%Y-%m-%dT%H:%M:%SZ)

# Real newlines, built with char(10). The sqlite3 CLI does not interpret `\n` inside a
# string literal, so writing them literally stores the two characters backslash and n —
# which render in the UI as `merged.\n*Slack channels*` and read as a bug in the app.
nl="' || char(10) || '"

# Off by default in the sqlite3 CLI, and a seed that orphans rows would look like a
# working demo.
#
# (job_id, tick) is UNIQUE — that is the duplicate-due-time guard — so the ticks differ.
sqlite3 "$db" <<SQL
PRAGMA foreign_keys = ON;

INSERT INTO jobs (name, schedule, channel_id, channel_name, context_window, prompt_template, enabled)
VALUES
  ('Day Task', '{"kind":"daily","at":"09:30","tz":"Asia/Kolkata"}', 'C0DAILY1', 'coot-ai',
   '{"kind":"since","at":"18:30","previous_day":true,"tz":"Asia/Kolkata"}',
   'What I plan to do today. Three bullets.', 1),
  ('Progress Update', '{"kind":"daily","at":"14:30","tz":"Asia/Kolkata"}', 'C0PROG02', 'coot-ai',
   '{"kind":"lookback","hours":6}',
   'What moved since the last post. Name the thing, not the effort.', 1),
  -- Deliberately NOT the machine's zone: this Job's window must print in Pacific time
  -- even when the machine is in India, because 09:30 is 09:30 in the Job's own zone.
  ('Day Summary', '{"kind":"daily","at":"18:30","tz":"America/Los_Angeles"}', 'C0SUM003', 'coot-ai',
   '{"kind":"since","at":"09:30","previous_day":false,"tz":"America/Los_Angeles"}',
   'What actually shipped today, and what is still open.', 1),
  ('Weekly Retro', '{"kind":"daily","at":"17:00","tz":"Asia/Kolkata"}', 'C0WEEK04', 'eng-platform',
   '{"kind":"lookback","hours":24}', 'Held for now.', 0);

INSERT INTO fires (job_id, fired_at, outcome, no_signal, error, tick) VALUES
  (2, '$now', '"drafted"', 0, NULL, 11),
  (3, '$now', '"partial"', 0, NULL, 12),
  (1, '$now', '"missed"',  1, NULL, 13),
  (2, '2026-10-02T09:05:00+00:00', '"failed"', 0,
   'slack: invalid_auth (the d cookie expired when you logged out of Slack)', 14),
  (3, '2026-10-02T08:55:00+00:00', '"drafted"', 0, NULL, 15);

INSERT INTO drafts (job_id, fire_id, text, window_from, window_to, counts, no_signal, partial,
                    parent_id, discarded, created_at, edited, approved, approved_at)
VALUES
  (2, 1,
   '*Auth rewrite*: the token path now validates against one helper, so there are three call sites instead of six. #482 is merged.$nl*Slack channels*: listed and joined #eng-platform so the retro has somewhere to land.$nl*Next*: the retry backoff, which is still untested under load.',
   '2026-10-02T13:01:00+00:00', '2026-10-02T19:01:00+00:00',
   '{"slack_messages":12,"slack_channels":2,"github_comments":4,"github_repos":1,"slack_fetched":38,"github_fetched":9}',
   0, 0, NULL, 0, '$now', 0, 0, NULL),
  (3, 2,
   'GitHub could not be read for this window, so this covers Slack only.$nl$nl*Shipped*: the settings screen, behind the checklist.$nl*Not shipped*: the channel picker. It is written and unverified.',
   '2026-10-02T09:30:00+00:00', '2026-10-02T19:01:00+00:00',
   '{"slack_messages":6,"slack_channels":1,"github_comments":0,"github_repos":0,"slack_fetched":6,"github_fetched":40}',
   0, 1, NULL, 0, '$now', 0, 0, NULL),
  (1, 3, 'NO SIGNAL', '2026-10-02T04:00:00+00:00', '2026-10-02T19:01:00+00:00',
   '{"slack_messages":0,"slack_channels":0,"github_comments":0,"github_repos":0,"slack_fetched":0,"github_fetched":0}',
   1, 0, NULL, 0, '$now', 0, 0, NULL);

SELECT 'jobs: ' || count(*) FROM jobs;
SELECT 'fires: ' || count(*) FROM fires;
SELECT 'drafts: ' || count(*) FROM drafts;
SQL