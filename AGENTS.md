# SlackBot

A local Rust daemon that drafts Slack status posts from your real activity —
your Slack messages and your Git history — on a schedule, and posts them only
when you click Approve.

`cargo run` starts an Axum server on `127.0.0.1:7317` and opens a browser UI.
You configure the LLM, Slack credentials, and your posting routine once; after
that it drafts. Nothing is ever posted without an explicit click.

See [`idea.md`](./idea.md) for the full spec.

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