# SlackBot

Drafts the status messages your team expects — a morning task list, a midday progress
note, an end-of-day summary — from your own Slack messages and your own GitHub
comments, on a schedule you set once.

**It posts nothing you have not asked for.** By default every message waits in the Inbox
until you click Approve. One switch in Setup turns on auto-send, and then each Draft goes
out as it is written. It is off until you turn it on, and nothing else can (ADR-0010).

```
cargo run
```

Opens `http://127.0.0.1:7317` in a browser. That is the whole install.

---

## What it does

Three times a day, for each Job you configure:

1. **Collect.** Reads your Slack messages and your GitHub comments from the Job's
   window. Nothing is written to disk — the activity is rendered once, used, and
   discarded.
2. **Draft.** Sends that to your chosen model and receives a post in Slack's format.
3. **Wait.** The Draft appears in the Inbox. You read it, edit it, and either Approve
   it or Discard it.

`deliver` is the only path to Slack that exists in the code, and a test asserts that
exactly two things reach it: a click on Approve, and auto-send.

Auto-send never posts a Draft whose window collected nothing, or one that is missing a
source. Those stay in the Inbox for you, because a broken window is not a status update.

## Setup

The setup screen is one board with three rows, each verified on its own:

| Row | What it needs | Where it goes |
|---|---|---|
| `LLM` | an API key, an endpoint, a model name | the key in your OS keychain; the endpoint in a local config file |
| `SLK` | an `xoxc-…` token and its `d` cookie | the keychain |
| `GH` | a token with read access | the keychain |

Then describe your routine in your own words and check the proposed Jobs before
saving them. The model proposes the times, the channels, and the prompt for each Job;
nothing is scheduled until you confirm it.

### The Slack cookie

Paste the `d` cookie **exactly as your browser sends it**, percent-escapes and all.
The tool sends it verbatim. Decoding it is the most common way to break a setup that
looks correct, and the resulting error reads like a wrong token rather than a mangled
cookie.

The cookie dies when you log out of Slack. When it does, the tool says so plainly and
tells you what to re-paste — it never silently drafts from zero Slack messages.

## Why the session token and not an OAuth app

An app would tag every post with a visible "sent via a bot app" line in a channel your
colleagues read, which defeats the point of a post that is meant to read as yours. The
session token posts with your real identity and no visible artifact.

The cost: it is undocumented, and it cannot be used to delete a message you have
already sent. `docs/adr/0002-slack-session-token-auth.md` records the trade in full,
including what happens when it breaks.

## Slack's terms, if you run this anywhere but your own machine

Slack's API terms restrict using API data to train a large language model and restrict
bulk export of message data. Feeding your own Slack history to a model about your own
work is not covered by that clause as written. **Running this against someone else's
workspace, or sharing it with your team, puts them in the clear.** The tool is built as
a personal, single-user tool on purpose.

## Requirements

- Rust 1.75 or newer
- A Slack session, a GitHub token, and an OpenAI-compatible endpoint

Node is **not** needed to run this. The UI is pre-built and committed to `dist/`, embedded
in the binary at compile time. Node is only needed to *change* the UI.

On Windows, building requires Visual Studio Build Tools (bundled SQLite is compiled from
source). macOS and Linux need nothing extra.

## Commands

| Command | What it does |
|---|---|
| `cargo run` | The app. Opens the board in a browser. |
| `cargo test` | 107 unit tests. No network, no keychain, no Slack. |
| `cargo test --test e2e -- --ignored` | 19 HTTP tests. Needs a running server. |
| `cargo clippy --all-targets` | Lints. CI treats warnings as errors. |
| `npm --prefix ui run build` | Rebuild the UI into `dist/`. **Then `cargo build`** — the binary embeds `dist/` at compile time, so skipping it serves the previous UI. |
| `npm --prefix ui run verify` | 34 browser checks at three widths. Needs a running server. |
| `scripts/seed-demo-db.sh <db>` | Fill a throwaway database with Jobs, Fires and Drafts, so the UI can be looked at without real credentials. |

CI also fails a pull request whose committed `dist/` does not match `ui/src` (ADR-0009).

## Architecture

A single Rust binary on loopback, SQLite for state, secrets in the OS keychain. Ten ADRs
in `docs/adr/` record the decisions that were expensive to reverse; the three that most
often surprise a reader:

- **0001 / 0010** — what reaches Slack, and why auto-send exists but is off by default.
- **0002** — a session token rather than an OAuth app, and what that costs.
- **0006** — why message content is never written to disk.

`docs/UX.md` is the user flow and why each step is shaped the way it is. `DESIGN.md` is
the visual system. `GLOSSARY.md` defines the domain words — use those.

## Where things live

| Path | What it holds |
|---|---|
| `src/domain.rs` | Job, Fire, Draft, Context Window, Schedule |
| `src/store.rs` | SQLite: Jobs, Fires, Drafts |
| `src/slack.rs` | Slack reads and the one write |
| `src/github.rs` | GitHub comments, and repo discovery |
| `src/evidence.rs` | Turning activity into the text the model reads |
| `src/llm.rs` | The model client, and the two separate roles |
| `src/scheduler.rs` | The Fire loop |
| `src/routes.rs` | The HTTP surface, and `deliver` — the only function that reaches Slack |
| `ui/src/` | The React board |
| `dist/` | The built UI. **Committed**, and embedded by `build.rs` (ADR-0009) |
| `build.rs` | Walks `dist/` and generates the embedded asset table |

`docs/adr/` records the decisions that are hard to reverse, and `GLOSSARY.md` the
vocabulary. `AGENTS.md` points at both and says which wins.
