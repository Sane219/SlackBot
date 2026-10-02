# A Fire either produces a Draft or records why it did not

Every time a Job comes due, exactly one `Fire` row is written, with an outcome. The
outcomes are `drafted`, `partial`, `failed`, `skipped` and `missed`. There is no silent
path: a scheduler tick that does anything at all writes a row.

## Why

ADR-0001 makes a missed Fire a visible gap rather than a non-event, because silent
failure is the single outcome that makes an unattended tool untrustworthy. That promise
is only keepable if every terminal state is recorded, including the ones where nothing
went wrong in the usual sense — a laptop asleep, an app that was not running.

## Consequences

**Sleep skips, it does not catch up.** A Fire whose time passed while the process was not
running is `missed`, listed in the UI on next start, and never retro-fired. The user chose
"keep the app running" over catch-up, and a catch-up burst on waking would produce three
stale posts at once.

**Partial is a real outcome, distinct from failed.** When Slack fails and GitHub
succeeds, the Fire is `partial`: the Draft is produced from what arrived and is labelled
partial in the UI. This is preferred to failing outright, because a partial day is still
worth a draft. The Danger is the model filling the Slack-shaped hole with invented Slack
activity, so partial Evidence carries an explicit statement of which source is absent.

**`skipped` is for duplicate due-times**, not for user preference. Two Jobs sharing a
minute both fire; a Job that is disabled writes nothing at all, because a disabled Job
having "skipped" is misleading.

**The Fire log is bounded.** Drafts are kept until the user acts on them or 30 days pass.
Fire rows are kept 90 days. Both are pruning, not expiry — the UI shows an Inbox, and an
Inbox that accumulates forever stops being readable.

Ticking is once per 20 seconds. At 20-second granularity two Jobs cannot share a minute
without sharing a tick, so the `skipped` outcome exists only for the case where a Job's
clock lands within the same 20 seconds as the tick that already processed it.