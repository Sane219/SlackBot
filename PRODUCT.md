# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

The tool is a local server with a browser UI. The server binary is native and must run
on macOS, Linux, and Windows; the UI is plain HTML/CSS/JS served from that binary, with
no framework and no build step.

## Stack

Rust, single crate. Axum 0.8 + Tokio + serde for the server; SQLite for Jobs, Drafts and
the Fire log; `open` for the browser launch; OS keychain for secrets (needs a per-platform
abstraction to cover macOS, Linux and Windows). UI is vanilla HTML/CSS/JS embedded in the
binary at compile time, so `cargo run` works from any directory.

## Users

One person per install. Primary users are engineers, project managers, and manual
testers at a software company, working at a laptop during their day.

PMs and testers are a first-class audience, not an edge case. Their work is recorded as
GitHub comments, PR reviews and board moves rather than commits, so evidence collection
must lead with GitHub and treat local git as a fast path where a repository happens to be
checked out.

## Product Purpose

Drafts the recurring status posts a team expects — a morning post about intentions, a
midday progress post, an end-of-day summary — from the user's own real activity in Slack
and GitHub, three times a day, on a schedule the user configures once.

Success: three Drafts a day that need under 30 seconds of editing before they are good
enough to send, and a Day Summary indistinguishable in style from one written by hand.

## Positioning

The reconstruction is mechanical and the judgement is human, so the tool does only the
first half. It gathers evidence from the places the work actually happened, renders it
once, and proposes a post — and by default a human presses the button.

The differentiator is that the judgement stays explicit by default and is a one-time
choice when you want it otherwise. Neighbouring tools (cron-plus-LLM scripts, Slack bots,
status-report generators) auto-send with no option. This one waits for a click, and offers
auto-send as a switch you deliberately turn on (ADR-0010) — never as the default, and
never for a window that collected nothing or is missing a source. Both paths go through
`deliver`, the only function that reaches Slack, which is a tested property rather than a
convention.

## Operating Context

Users run `cargo run`, which binds `127.0.0.1:7317` and opens a browser. The process is
the daemon: closing the terminal stops scheduling, and missed Fires are shown on next
start rather than caught up. A laptop asleep at a Fire time skips that Fire.

Three credentials are configured once at setup: an LLM API key, a Slack session token
plus its `d` cookie, and a GitHub token. All three live in the OS keychain, never in a
config file.

Authenticating to Slack uses an undocumented session-token path rather than an OAuth app.
This is deliberate: an app would tag every post with a visible "sent via a bot app"
artifact in a channel colleagues read. The accepted cost is that the cookie dies on Slack
logout and must fail loudly with re-auth instructions.

## Capabilities and Constraints

- Slack reads and writes go through the session token. `conversations.history` and
  `.replies` are Tier 3 (50+/min); no rate-limit headers are published for this token, so
  collection paces itself at roughly 1 req/sec.
- GitHub evidence comes from per-repo `/issues/comments` and `/pulls/comments` with
  `since`, not from search: the `commenter:` qualifier matches issue comments only and
  misses the review comments that constitute a reviewer's actual work. `since` filters on
  `updated_at`, so an edit to an old comment resurfaces it and needs in-process filtering.
- No raw Slack messages or GitHub comments are persisted to disk. Evidence is rendered
  per Fire, used, and discarded.
- The tool holds no names or shapes for a team's status messages. Both are learned from
  the user's own description at setup and stored per Job, because teams differ.
- The server binds loopback only. It serves a local API with no authentication, which
  would include the Approve endpoint, so a non-loopback bind is refused.
- Single person per install: one Slack identity, one GitHub identity, configured once.

Undecided: whether different Jobs may use different models, and what happens to Draft
lineage when they do.

## Brand Commitments

No established visual identity. The name is SlackBot.

## Evidence on Hand

No real product content exists yet. There is no customer, no usage data, no screenshot of
a team posting, and no testimonial. Any Draft shown in a demo or a placeholder must be
labelled synthetic. Nothing about adoption, accuracy, or time saved may be claimed.

## Product Principles

1. **The judgement stays explicit.** By default a human sends. Auto-send exists as a
   deliberate one-time choice (ADR-0010), and it still refuses a gap or a partial Draft.
   `deliver` is the only path to Slack either way, and that is a property of the code
   rather than a preference.
2. **Say nothing rather than invent.** A thin day produces a visibly thin Draft. ADR-0003
   requires the morning post, whose evidence is weakest, to be willing to say nothing.
3. **A failure must be visible.** A missed or broken Fire shows as a gap in the Inbox.
   Silent failure is the single outcome that makes an unattended tool untrustworthy.
4. **Learn the user's practice, never impose one.** No house vocabulary, no seeded
   templates, no assumed working hours.
5. **Keep company content off disk.** Credentials in the keychain, evidence rendered and
   discarded.

## Accessibility & Inclusion

WCAG AA. Keyboard navigation, visible focus, sufficient contrast, and real form labels
are required, not best-effort. The tool is used repeatedly through a working day, often
by someone rushing between other tasks.