# ADR-0009 — The UI is React, built ahead of time and committed

The front end is React, built with Vite to `dist/`, committed to the repository, and
embedded in the Rust binary with `include_str!`. There is no build step at run time.

## Why

The original spec committed to "no `node_modules`, no framework to version. `cargo run`
must mean `cargo run`." That was the right call for a UI that never grew past three
tabs and a form.

It has grown. The app now has a job editor with a channel picker, a three-step setup
checklist, an Evidence disclosure on every Draft, a next-fire row on the spine, and live
state across all of it. Hand-written DOM manipulation is now the thing costing us
correctness: the bug where `display: flex` beat the `hidden` attribute is a symptom of
managing visibility by hand, and there are more of those in a bigger UI than in a smaller
one.

The commit-and-embed split keeps the promise that actually matters. `cargo run` still
means `cargo run` — no `npm install`, no Node on the run path. Only *changing* the UI
requires a build, and that is a contributor-time cost rather than a user-time one.

## Consequences

**The committed `dist/` is a contract.** A pull request that changes `ui/src` without
regenerating `dist/` is wrong, and CI checks it: a Node step builds and fails if the
output differs from what is committed. Rust embeds the whole tree through `build.rs`,
which walks `dist/` and emits a lookup table.

This has teeth, and they were checked rather than assumed. Three things had to be true at
once, and two of them were false first:

- `build.rs` has to re-run when a file *inside* `dist/` changes. Cargo does not track
  directory contents, so it emits `rerun-if-changed` per file it embeds.
- The URL a file is served at has to match what `dist/index.html` asks for. A first
  version published `dist/assets/app.js` as `/assets/assets/app.js`, and then
  `//app.js`; both served 200 with a blank page.
- A `dist/` that is missing must not break `cargo run`. An empty one embeds a page that
  says what to run, because a compile error would break the one promise this ADR exists
  to keep.

The e2e suite reads the asset paths out of the served page rather than hardcoding them,
so a bundle that exists but is not referenced cannot pass.

**The binary must be rebuilt for a UI change to appear.** `include_str!` reads `dist/`
at compile time, so `npm run build` without `cargo build` serves the previous UI. That
is the intended trade: the alternative — reading `dist/` at runtime — would put a
directory next to the binary and break "needs nothing beside it". It does mean the
development loop is two commands.

**Node is a development dependency, not a runtime one.** `rust-version` and the MSRV are
unaffected. A user installing the binary needs no Node at all.

The single-page world is unchanged: this is still a local server, still loopback-bound,
still embedding its assets. Only the authoring language of the front end moved.

What this decision does *not* do is fix the current UI's problems. Those are in
`docs/UX.md`, which was decided separately.

## Verifying a change

Three scripts, all run against a server on `:7321`. None of them is a substitute for
looking at the page — a bug where a class's `display: flex` beat the `hidden` attribute
was invisible to a DOM probe and obvious in a screenshot — but they make the things a
probe cannot see cheap to re-check.

- `node ui/verify.mjs` — 34 assertions at three widths: no horizontal overflow, exactly
  one view visible, every spine row shows a time, every tab stop has a focus ring, the
  floating Setup button hides no text at rest, no console errors.
- `node ui/shot.mjs <label> --tab=… --w=…` — screenshots for a human.
- `node ui/drive.mjs` — drives the interactive paths (edit, evidence, approve, add a Job
  by hand) and reports what changed in the DOM.

`scripts/seed-demo-db.sh` fills a throwaway database with Jobs, Fires and Drafts, so the
states that matter can be looked at without real credentials. It writes JSON into the
columns that hold JSON — a bare `drafted` where `fires.outcome` expects `"drafted"` — and
that mistake cost an afternoon, because every decode failure reported itself as
`rusqlite::Error::InvalidQuery`, whose Display is **"Query is not read-only"**.
