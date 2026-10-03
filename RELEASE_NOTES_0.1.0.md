## Tags

`status:pre-release` · `type:feature` · `platform:macos` · `platform:linux` · `platform:windows`

Pre-release because **no Fire has run against real Slack and GitHub credentials yet**.
The code, the tests and the UI are done; the first real run is not. `platform:windows`
is marked on the strength of a single `cargo check --target x86_64-pc-windows-msvc`,
not a CI job.

`platform:macos` and `platform:linux` mean verified to build and run there. Windows needs
Visual Studio Build Tools, so it is one prerequisite away from the same status.

## The first version you can run

`cargo run`, and a browser opens. That is the whole install — no Node, no Docker, no
service to configure.

On macOS and Linux as-is. On Windows you need Visual Studio Build Tools, because the
embedded SQLite is compiled from source.

## What it does

Three times a day, for each Job you set up:

1. **Collect** — reads your Slack messages and your GitHub comments from that Job's
   window. Nothing is written to disk; the activity is rendered once, used, and discarded.
2. **Draft** — that rendering goes to your model, which returns a post in Slack's format.
3. **Wait** — the Draft sits in the Inbox until you click Approve.

You describe your routine in a sentence and the model proposes the schedule, or you add a
Job by hand. Either way you confirm before anything is scheduled.

## Read this before you connect Slack

Authentication is an `xoxc-…` user token plus the session's `d` cookie. This is deliberate
([ADR-0002](docs/adr/0002-slack-session-token-auth.md)): an OAuth app tags every post with
a visible "sent via a bot app" line in a channel your colleagues read.

Two consequences you should know about:

- **It is undocumented.** If Slack changes the cookie contract, this breaks, and the error
  will look like a wrong token.
- **There is no delete.** `chat.delete` is unavailable on this path, so **a sent message
  cannot be recalled.** This is the single largest reason the default is a click.

Slack's API terms also restrict running this against anyone else's workspace. It is a
personal tool on purpose.

## Auto-send

One switch in Setup, off until you turn it on ([ADR-0010](docs/adr/0010-auto-send.md)). When
on, each Draft posts itself as it is written.

It still refuses two things: a window that collected nothing, and a Draft missing a source.
Those stay in the Inbox, because a broken window is not a status update and its own text
says a source failed. A gap never becomes a post under your name.

If auto-send skips something, the Draft is simply still there when you open the board.

## Known gaps

- **No Fire has ever run against real Slack and GitHub credentials.** Everything is
  verified against mocks and a seeded database. The first real run is untested.
- No Windows CI job. The cross-platform claim was checked once by hand.
- Not yet run through `impeccable detect`; it was not on `PATH` during the UI rewrite.
- The comp round behind `DESIGN.md` was unavailable, so the visual system was derived from
  the built CSS rather than compared against a rendering.

## Where to look

- `docs/UX.md` — the flow, and why each step is shaped that way
- `DESIGN.md` — the visual system, with named rules that are load-bearing
- `docs/adr/` — ten decisions, each with what was rejected and why
- `GLOSSARY.md` — the domain vocabulary

## Checks

```sh
cargo test                                    # 107 unit
cargo test --test e2e -- --ignored            # 19 HTTP, needs a running server
npm --prefix ui run verify                    # 34 browser checks, needs a running server
```

The unit tests never touch the network, your keychain, or Slack.
## Building from source

No release artifacts are attached: this is a Rust program with an embedded UI, and the
whole install is `cargo run`. Cloning and building is the install.

```sh
git clone https://github.com/Sane219/SlackBot
cd SlackBot
cargo run
```

`dist/` is committed, so there is no Node step. `cargo run` opens `http://127.0.0.1:7317`.

Windows needs Visual Studio Build Tools for the bundled SQLite. macOS and Linux need
nothing beyond a Rust toolchain.
