---
version: 1
slug: "src-web-index-html"
primary_target: "src/web/index.html"
related_targets: []
---

# Setup + Inbox surface

## Scope and mode

Operate. Setup (first-run credential capture) and Inbox (Draft review) for a single
person per install. These are the two surfaces a user meets every day.

## Audience and job

One person, at their laptop, three times a day. Job: review a drafted status post and
either approve it for sending or edit it. Frequency is high, attention is low, and the
user is usually between other tasks.

## Action and proof

The Approve button is the one action that reaches Slack, so it must be unmistakable and
must never sit adjacent to Discard. Proof of correctness is the user's own Evidence:
the Draft's Context Window, its source counts, and its lineage back to the previous
Draft.

## Constraints

WCAG AA. Loopback-only server. Three credentials captured once: LLM API key, Slack
session token + `d` cookie, GitHub token. Evidence is never persisted, so any screen
showing what a Draft was drawn from can only show counts and the window, not the source
text.

## Chosen direction

The incident postmortem wall. Near-black ground, grid ruled in dim cyan, one signal-amber
trace. A day is a sequence, so the timeline is the primary axis running down the left;
the Draft lives in the wide right pane. Column codes (`ACT` `MSG` `GIT` `PR` `SYS`) let
the eye scan before it reads.

Memorable moment: **the gap is drawn as a gap.** A window with no Evidence is a hatched
void with its span printed across it, reading `NO SIGNAL 14:30–18:30`. This is what
distinguishes *nothing happened* from *we did not look*, and it is product principle 2
made visible.

## Direction contract

THESIS: An operations board that could shout would be a board you could no longer
trust. Refuses the dashboard reflex — no cards, no glow, no status pills — in favour of
one axis, one pen, and honest voids.

OWN-WORLD: Near-black incident wall (#0a0b0d), grid ruled in dim cyan (#1b3a3f), trace in
signal-amber (#ffb020), vermilion (#ff3b30) reserved solely for a failed Fire's `SEV1`.
One condensed grotesque at one size, whole surface. Rank by weight and rule only.

STORY: The user sees their day as a recorded timeline. Rows are Fires in schedule order,
the morning post above the summary it seeds. A gap is drawn as a gap. The Draft waits in
the right pane until they approve it.

FIRST VIEWPORT: Timeline strip at 180px on the left carrying the incident clock
(`09:00 / 14:30 / 18:30`) and one row per Fire. Right pane holds the newest Draft, its
window printed as an edge scale, source counts beneath, and the action row at the
baseline — Approve and Discard separated by a full rule, never adjacent.

FORM: Assigned direction, position 3 of the grounded list, seed key 1d97c764. Raised by
four declined challengers: the lexicon's guide words become the incident clock; the
specimen's grain honesty makes truncated Evidence say so in the margin; the sleeve's
code discipline means state speaks in code and nothing is labelled twice; the split-flap
cascade collapses to one restrained turn per state change.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review,
the verdict, DESIGN.md, and every shipping raster carrying its provenance.
