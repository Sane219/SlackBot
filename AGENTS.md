# SlackBot

A local Rust daemon that drafts Slack status posts from your real activity — your
Slack messages and your work recorded in GitHub — on a schedule, and posts them
only when you click Approve.

`cargo run` starts an Axum server on `127.0.0.1:7317` and opens a browser UI.
You configure the LLM, Slack credentials, and your posting routine once; after that
it drafts. Nothing is ever posted without an explicit click.

## Before designing anything

Read `docs/adr/` first. **Where an ADR and `idea.md` disagree, the ADR wins** —
`idea.md` is the record of the original thinking, not current truth, and it carries
a header mapping each reversal.

## Before touching the UI

`DESIGN.md` is the visual system, written from the built CSS and verified token by
token. Its named rules are load-bearing, not descriptive: **One Voice Rule** (amber on
under 5% of a screen), **Vermilion Rule** (a failed Fire and nothing else), **One Size
Rule** (a second font size is a system bug), **Flat-By-Default** (nothing lifts or
glows), **AA Floor** (every ink clears 4.5:1 against its real ground).

Run `impeccable detect` on changed UI files. One finding is deliberate and documented in
`.impeccable/BLOCKER.md`: the hatch's repeating gradient *is* the gap.

The live design was worked as a wayfinding map at
[#1](https://github.com/Sane219/SlackBot/issues/1).

## Vocabulary

`GLOSSARY.md` defines Draft, Fire, Job, Context Window, Evidence and Approve. Use
those terms. The tool imposes no names or shapes for a team's status messages
(ADR-0004) — those are learned per user and stored per Job.

## Before changing the UI

`ui/` is a React app built by Vite to `dist/`, which is **committed** and embedded in
the binary by `build.rs`. So a UI change is two commands:

```sh
npm --prefix ui install && npm --prefix ui run build   # after changing ui/src
cargo build                                           # or cargo run
```

Skipping the `cargo build` serves the *previous* UI: `include_str!` reads `dist/` at
compile time. CI fails a pull request whose `dist/` does not match its sources.

Verify in a real browser, against a server on `:7321`:

```sh
scripts/seed-demo-db.sh "$SLACKBOT_DATA_DIR/slackbot.db"   # throwaway data to look at
node ui/verify.mjs        # 34 assertions at 1440 / 1024 / 390
node ui/shot.mjs label --tab=jobs --w=390 --h=844 --full
```

**Screenshots, not DOM assertions.** A probe once reported `hidden` as set correctly
while the element was plainly on screen, because a class's `display: flex` beat the
attribute. Compute styles and pixels, and only pixels settle it.

Two traps in this codebase that have each cost real time, both now pinned by tests:

- A missing `#[serde(default)]` turns a partial request body into a 422 before any
  validation runs. Three separate bugs were this one.
- `rusqlite::Error::InvalidQuery` renders as **"Query is not read-only"**. It is never a
  permission problem; it is always a value this code could not decode.

## Agent skills

### Issue tracker

GitHub Issues, via the `gh` CLI. This repo lives under the `Sane219` account.
See `docs/agents/issue-tracker.md`.

### Triage labels

Five canonical roles with default names: `needs-triage`, `needs-info`,
`ready-for-agent`, `ready-for-human`, `wontfix`.
See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` and one `docs/adr/` at the repo root.
See `docs/agents/domain.md`.