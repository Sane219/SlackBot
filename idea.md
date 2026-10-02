# slackbot — Idea Spec

> **This document is superseded. Where it disagrees with `docs/adr/`, the ADR wins.**
>
> Kept as the record of the original thinking — what was argued, and why the design
> moved. Four decisions below were reversed during design; the reasoning survives in
> their placeholders so the reversal is auditable rather than looking like a mistake.
>
> Current decisions, in one place:
>
> | This document says | The ADR says |
> |---|---|
> | §6, §12: local `git log` is the primary source, GitHub optional | `0003`: GitHub leads, local git is the exception, repos discovered from activity |
> | §11: seeded defaults for 09:30 / 14:30 / 18:30 and three canonical message shapes | `0004`: the tool imposes no vocabulary; names and shapes are learned per user |
> | §7: honor `Retry-After` on Slack rate limits | No rate-limit headers are published for `xoxc` — pacing must be self-imposed |
> | §2: drafts from your own activity; §5 has no recall concept | `0001`: Approve is the only path to Slack; `0002`: no recall, because `chat.delete` does not work on the session-token path |
>
> Two further facts were established by measurement and contradict nothing here but
> are worth knowing: Slack's 1 req/min non-Marketplace penalty does **not** apply to a
> session token (it is scoped to distributed apps), and GitHub's `commenter:` search
> qualifier matches issue comments only — a reviewer's review comments are invisible to
> it. See ticket [#7](https://github.com/Sane219/SlackBot/issues/7).

A local Rust daemon that drafts your recurring Slack status posts by feeding your
real activity to an LLM on a schedule, and posts them only when you click Approve.

Status: **Idea, pre-implementation.** No code written yet. The live design is being
worked as a wayfinding map at
[#1](https://github.com/Sane219/SlackBot/issues/1).

---

## 1. Problem

Every engineer at our company posts a Day Task, a Progress Update, and a Day
Summary. Today that means:

- opening Slack and scrolling back to reconstruct what you did
- opening GitHub or `git log` for the other half
- hand-writing three structured posts in the team's exact format
- repeating that three times a day

The reconstruction step is mechanical. That is the part a tool should do. The
*judgement* step — what actually matters, what to ask for, what to promise
tomorrow — should stay with the human. Hence: **the tool drafts, the human
sends.**

## 2. Non-goals

- **Never auto-post.** No message leaves the machine without an explicit click.
  This is a hard rule, not a default.
- **Not a Slack app.** No bot user, no OAuth install, no workspace admin.
  Everything runs locally against your own user session.
- **Not a team tool.** Single user, single machine. No multi-tenant, no server.
- **Not a generic note app.** It knows about Slack and Git. That is the scope.
- **Does not post to `#general`.** Channel is whatever the job says, and the
  job's channel is configured by the user.

## 3. The flow

```
cargo run
   │
   ├─ Axum binds 127.0.0.1:7317, prints URL, opens Chrome
   │
   ▼
┌─────────────────────────────────────────────────────────┐
│  Setup (first run only)                                 │
│  1. LLM: base URL, model name, API key                  │
│  2. Slack: xoxc token + d cookie                        │
│  3. Git: which repos to read                             │
│  4. Describe your posting routine in plain English      │
└─────────────────────────────────────────────────────────┘
   │
   ▼  "Submit"
   │
   ├─ LLM call #1 → returns a ScheduleSpec as JSON
   ├─ Validate against schema, render a readable preview
   ▼
┌─────────────────────────────────────────────────────────┐
│  Confirm schedule                                       │
│  Day Task     09:30 IST  #coot-ai   enabled             │
│  Progress     14:30 IST  #coot-ai   enabled             │
│  Day Summary  18:30 IST  #coot-ai   enabled             │
│  [ Save and start ]                                     │
└─────────────────────────────────────────────────────────┘
   │
   │  ... time passes, daemon sleeps ...
   ▼
   14:30 IST fires
   ├─ Fetch Slack history for the job's window
   ├─ Run git log across configured repos for that window
   ├─ Render prompt template with that context
   ├─ LLM call #2 → draft in Slack mrkdwn
   └─ Draft lands in the Review inbox
   │
   ▼
┌─────────────────────────────────────────────────────────┐
│  Review                                                 │
│  Day Task · yesterday 3 messages, 2 commits, 1 PR       │
│  ┌───────────────────────────────────────────────────┐  │
│  │ Day Task:                                         │  │
│  │ • *Failover retry*: add jitter to reconnect path  │  │
│  │ • *Indexer*: chunker benchmark                    │  │
│  └───────────────────────────────────────────────────┘  │
│  [ Approve & Send ]  [ Edit ]  [ Regenerate ]  [ Discard ]│
└─────────────────────────────────────────────────────────┘
```

Editing before send is expected, not an escape hatch. The LLM gets you 90% of
the way; you fix the sentence.

## 4. Two distinct LLM roles

This is the core design decision. The setup call and the drafting call have
completely different contracts and must not share a prompt.

**Role A — Plan (runs once at setup, re-runnable)**

- **Input:** free-text description of the routine + the team's real templates
  (seeded from `slack_status_templates` so the defaults match house style).
- **Output:** strict JSON conforming to `ScheduleSpec`.
- **Posture:** the model *proposes*, the user *confirms*. Nothing is scheduled
  without a click on the rendered preview.
- **Failure mode:** if the JSON does not deserialize, or contains times the
  `cron` parser rejects, show the error and the raw output. Never auto-repair
  into a working schedule.

**Role B — Draft (runs on every fire)**

- **Input:** system prompt (persona + tone + hard formatting rules) + the job's
  prompt template + rendered activity context.
- **Output:** plain Slack mrkdwn text. No JSON, no preamble, no code fence.
- **Posture:** deterministic-ish. Same context, same temperature, roughly same
  draft. Never posts.

Keeping these separate means a hallucinating draft model can never invent a
schedule, and a rambling setup model can never produce a half-formed post.

## 5. Job model

```rust
struct JobSpec {
    id: JobId,
    name: String,                    // "Day Task"
    schedule: Schedule,
    channel: ChannelRef,             // { id: "C0A0RRC7P8B", name: "coot-ai" }
    context_window: ContextWindow,
    prompt_template: String,         // handlebars-ish, {{activity}}, {{history}}
    enabled: bool,
    last_fired_at: Option<DateTime>,
}

enum Schedule {
    Daily { at: LocalTime, tz: TzId },   // "09:30 in Asia/Kolkata"
    Cron  { expr: String,      tz: TzId }, // escape hatch
}

enum ContextWindow {
    Lookback { hours: i64 },             // last N hours of everything
    Since   { at: LocalTime, tz: TzId },// e.g. "since 18:30 yesterday"
    Explicit { from: DateTime, to: DateTime },
}
```

Notes on the model:

- **Timezone is per job, never global.** A scheduler that assumes the machine's
  timezone breaks the moment the laptop travels. IST is the default here, stored
  as a named zone.
- **Day Task is not a lookback job.** It is written in the morning about
  intentions, so its useful context is *yesterday's* work plus open items —
  `Since { at: 18:30, previous day }` reads better than `Lookback { hours: 12 }`.
  This is exactly the kind of thing the plain-English description should drive,
  and why Role A exists.
- **Context windows overlap deliberately.** The Day Summary window can start
  where the Day Task window ended, so the day's arc is reconstructable.

## 6. Activity collection

### Slack

Two calls, both needing **both** credentials — the `xoxc` token alone returns
`not_authed` for user-scoped conversation reads:

```
conversations.history  ?channel=C…&oldest=<unix>&latest=<unix>&limit=200
conversations.replies  for each message with reply_count > 0
```

- Auth headers: `Authorization: Bearer <xoxc>` **and** `Cookie: d=<xoxd>`.
- Paginate with `response_metadata.next_cursor` until exhausted, capped at a
  configurable page budget so a chatty channel cannot stall a fire.
- Slack rate limits are per-method (Tier 3/4 for these). Honor `Retry-After`.
- Messages are filtered to the user's own posts by `user == U…` before costing
  tokens — a fire should see *your* activity, not the channel's.
- Message bodies run through a small sanitizer: strip `<@U…>` mentions to
  readable names, `<https://…|label>` to `label`, `<!here>` → `@here`.
  Unescaped `&<>` will mangle mrkdwn if it reaches the model raw.

### Git

Local repos first, no token required:

```
git log --since=<iso> --until=<iso> --author=<email> \
        --pretty=format:%h|%an|%ad|%s --name-only
```

Repo roots are configured in setup. Commit subjects plus touched paths are
plenty for a status post; full diffs are not and would blow the context budget.

Optional GitHub provider later, for PRs and reviews — the abstraction is already
there via the `GitProvider` trait.

### Context assembly

Both sources become one bounded text block:

```
## Slack activity (2026-10-02 09:00 → 2026-10-02 14:30 IST)
- 10:12 #coot-ai sanket: pushed the jitter fix for the reconnect path
- 11:40 #infra sanket: PR #482 needs rebase, blocked on review
(31 messages by you in 4 channels)

## Git activity (same window)
- a3f9c12 failover: add exponential backoff to ws reconnect
  src/replica/reconnect.rs, src/replica/mod.rs
- 7b1e004 learn_rust: axum route extractor fix
  src/handlers/users.rs
(2 commits across 2 repos, 4 files touched)
```

Trim oldest-first when over budget, keeping counts accurate. A truncated context
that lies about its own size produces confidently wrong drafts.

## 7. LLM integration

OpenAI-compatible `/chat/completions` — works against OpenAI, OpenRouter,
Groq, Ollama, vLLM, and your own llama.cpp endpoint with no code changes.

- Key from the user, held in the OS keychain. Never written to config, never
  logged, never returned by any endpoint.
- **Explicit model choice.** The user types the model string. No auto-discovery,
  no silent fallback to a different model — a wrong model at a 2pm deadline is
  worse than a loud error.
- Small endpoint `/v1/models` probe at setup, so a bad base URL or key fails on
  the setup screen instead of at 14:30.
- Temperature low (0.2–0.4). These are factual posts.
- One retry on 429/5xx with backoff, then the fire is recorded as failed and
  surfaced in the UI. A missed draft is a visible gap, not a silent one.
- Token budget per fire is configurable, default ~12k, oldest-first trimming.

## 8. Architecture

Single crate, module-separated. Not a workspace: one binary, one install path,
no version skew between internal crates.

```
src/
  main.rs          arg parsing, server bind, browser launch
  config.rs        paths, defaults, TOML load/save
  secrets.rs       keychain wrapper; trait so tests can stub
  scheduler.rs     tick loop, next-fire computation, catch-up policy
  jobs.rs          JobSpec, Schedule, ContextWindow, CRUD
  llm/
    mod.rs         client trait + OpenAI-compatible impl
    plan.rs        Role A: description → ScheduleSpec
    draft.rs       Role B: context → message
  activity/
    mod.rs         ActivityEvent, ActivitySource trait
    slack.rs       history + replies, sanitizer
    git.rs         local git log
    assemble.rs    context building, trimming
  slack/
    client.rs      auth, rate-limit handling, paging
    send.rs        chat.postMessage
  store.rs         SQLite (or redb): jobs, drafts, fire log
  routes/          axum handlers
  web/             index.html + assets, vanilla JS
```

**Dependencies, kept deliberately few:** `axum` + `tokio` + `reqwest`,
`serde` + `serde_json` + `toml`, `chrono` + `chrono-tz`, `cron`,
`keyring`, `rusqlite` (bundled), `tracing`, `thiserror`, `handlebars`,
`open` (browser launch). Frontend is a few hundred lines of vanilla JS and CSS —
no build step, no `node_modules`, no framework to version. `cargo run` must mean
`cargo run`.

## 9. Scheduling semantics

A cron expression alone is not a product. Decisions worth making explicit:

| Situation | Behavior |
|---|---|
| Laptop asleep at fire time | Skip. Record `missed`. No catch-up burst on wake. |
| App not running at fire time | Next start shows missed fires in the UI; jobs do not retro-fire. |
| Two fires within the same minute | Second is skipped. |
| Slack fetch fails, LLM succeeds | Draft is still produced from whatever context was gathered, and labeled partial. |
| Slack fetch succeeds, LLM fails | Fire marked failed, error in UI. |
| Draft sitting unapproved at next fire | Kept, never auto-discarded. New draft arrives alongside. |

The through-line: **a failed fire must be visible in the UI.** Silent failure is
the one outcome that makes the tool untrustworthy.

## 10. Security

- **Bind `127.0.0.1` only.** Refuse `0.0.0.0`.
- **Session token** generated per run, injected into the opened URL as a
  fragment, required on every `/api/*` route. Defeats a random browser tab or a
  malicious page POSTing to the port.
- **Keychain for:** LLM API key, `xoxc`, `xoxd`. Plaintext config holds only
  non-secret settings (repos, channel ids, times).
- **Redaction** in `tracing` output for any field that can hold a token.
- **Slack writes are the only mutating call** in the entire app, and they only
  happen from the Approve handler. Worth a comment in the source, because it's
  the property everything else depends on.
- On request: refuse to run if a `SLACKBOT_*` env var would put a token in the
  process environment of a child process.

## 11. Seeded defaults

From the observed house style (`slack_status_templates`), offered as a starting
point the user edits rather than a blank form:

| Job | Time (IST) | Shape |
|---|---|---|
| Day Task | 09:30 | 3 bullets, `• *title*: explanation` |
| Progress Update | 14:30 | 3 bullets, last is `*Next / Ask*` |
| Day Summary | 18:30 | 5 bullets, ends `*Tomorrow*` |

Formatting rules that go into the system prompt as hard constraints:

- Bold is `*single asterisk*`. Not `**double**` — that renders literally.
- One bullet per line, `•` not `-`.
- Links as `<url|label>`, mentions as `<@U…>`.
- Issue and PR references inline: `#482`, PR link.
- No preamble, no "Here's your summary", no code fence around the output.

Small, but the difference between a post that looks native and one that looks
generated is entirely this list.

## 12. Phases

1. **Skeleton.** Axum server, config, keychain, SQLite, browser launch. A page
   that renders. Proves `cargo run` end to end.
2. **LLM client.** OpenAI-compatible, `/v1/models` probe, redaction, keychain.
   Setup screen stores and validates a real key.
3. **Activity collection.** Slack history + replies with both credentials,
   local `git log`, sanitizer, context assembly with trimming. Testable against
   a saved JSON fixture before any LLM is involved.
4. **Scheduler.** Tick loop, per-job timezone, the full miss/failure matrix from
   §9. `wiremock` for Slack.
5. **Role A + schedule UI.** Description in, validated `ScheduleSpec` out,
   rendered preview, confirm.
6. **Role B + Review inbox.** Draft generation, edit, approve-and-send,
   regenerate, discard.
7. **Polish.** SSE live updates, fire history, missed-fire banner, README,
   `cargo-dist` releases, CI.

Phases 1–4 contain no LLM in the critical path and are independently valuable.
Phase 3 is the one most likely to surprise: Slack's user-token auth is the
fiddliest integration here, and it should be proven early and against a fixture.

## 13. Risks

| Risk | Mitigation |
|---|---|
| Slack user-token read breaks on scope change | Detect `not_authed`, fail loudly with re-auth instructions, never silently degrade to zero context |
| Drafts are generic and get ignored | Seed templates from real house style; keep edit cheap; show the raw context alongside the draft so the user can see what it drew from |
| Users expect auto-post | Say so in the README and in the UI. The Approve button is the product |
| Context too large for cheap models | Trim oldest-first, cap tokens, keep counts accurate |
| Mac sleep kills the daemon | Explicit miss policy (§9); optionally a `launchd` plist in a later phase |

## 14. Success criteria

- `cargo run` on a clean machine reaches a working setup screen in one command,
  no second install step.
- A user sets it up once and never opens the config again.
- Three drafts per day appear on time, each needing under 30 seconds of editing
  before it is good enough to post.
- The Day Summary a user sends is indistinguishable in style from one written by
  hand.
- Nothing is ever posted without a click.