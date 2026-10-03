# ADR-0010 — Auto-send posts a Draft without a human

**Status:** accepted. Supersedes the "only a click reaches Slack" half of ADR-0001.

A Draft is posted as soon as it is written, when the user has turned auto-send on. It is
one switch in Setup, off by default, and nothing else in the app can turn it on.

## Why

ADR-0001 made approval the point of the tool: a wrong post under your own name is
unrecoverable, because there is no recall. That reasoning has not changed, and this ADR
does not argue against it.

The change is about friction. The tool is used three times a day on a routine the user
already knows, and reading a post you would have approved anyway is work with no upside.
The user asked for the click to go away and accepted the risk that comes with it.

## What was kept

ADR-0001's *shape* survives: `chat.postMessage` is still called from exactly one function.
That function is now `deliver`, and both a human Approve and auto-send go through it, so
the guard sequence — already-approved, empty, no-signal, claim, post, release — cannot be
duplicated and drift apart. `only_approve_reaches_slack` still asserts one call site, and
`only_approve_and_auto_send_reach_deliver` asserts that exactly two callers reach it.

## What auto-send will not do

Two refusals, both of which matter more with nobody watching:

- **It never posts a partial Draft.** `partial` means a source failed and the Draft says
  so in its own text. A human approving that is making a judgement; there is nobody here to
  make it, and the result names a defect in a team channel.
- **It never posts a gap.** A window that collected nothing is not a status update.

When it declines, it declines by **leaving the Draft in the Inbox**. That is why this needs
no new state, no new column, and no queue: the Draft simply waits, exactly as it would if
auto-send were off, and the user sees it. ADR-0002's rule — never degrade silently — is
satisfied by construction.

## Scope

One switch for every Job, not one per Job. A per-Job switch was the alternative and it is
the safer shape: a misclick sends one post rather than three. It was declined because the
routine is uniform — if the summaries are trusted at 18:30 they are trusted at 09:30 — and
because three switches on one screen is three chances to forget one is off.

**Ceiling:** if the routine ever splits — a boilerplate summary that needs no reading, and
a morning task that does — a single switch cannot express that, and one unattended Job
cannot be excluded. **Upgrade path:** a `jobs.auto_send` column, defaulting to the
install-wide flag so existing behaviour is unchanged, and a per-Job checkbox in the Jobs
tab. The scheduler already reads both from the Job, so the change is confined to the
lookup.

## Consequences

`FireRunner` holds an optional Slack client so the scheduler can post. It is not a second
writer: every send still goes through `deliver`.

A failed auto-send logs and returns. It does not fail the Fire and does not stop the
scheduler — `deliver` releases the claim on failure, so the Draft is still in the Inbox and
can be sent by hand. A Slack outage at 09:30 costs a log line, not a morning post.

The README and `docs/UX.md` said "it never posts on its own". Both were wrong the moment
this landed and have been corrected.