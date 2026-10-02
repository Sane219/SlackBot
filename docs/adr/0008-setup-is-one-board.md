# Setup is one board with three verified credential rows

The setup surface is the same incident board as the Inbox: a code column on the left
(`LLM`, `SLK`, `GH`) and the field on the right. No wizard, no stepper, no tabs. Each row
carries a live verification mark that changes only when the user asks for it.

## Why

The setup job is to answer one question — *what is configured and what is broken* — and a
screen that shows one credential at a time makes the user click through two of three to
answer it. Three credentials for one person is a list, and lists read better than steps.

The world also has an opinion about this: an operations board shows every channel's state
at once, because a responder who has to open a drawer to check one line does not check it.

## Consequences

Each credential verifies independently and asynchronously, so a GitHub failure does not
block testing an LLM key. A row's mark is one of `pending`, `ok`, or `failed` with a
short reason — state speaks in code, nothing is labelled twice.

Verification is deliberately cheap and separate from first use. At setup: an LLM
`/v1/models` probe, `auth.test` for Slack, `GET /user` for GitHub. Credentials that fail
here are not retried until the user asks, because a Fire that discovers a dead credential
at 14:30 has already wasted the window it was meant to cover.

Secrets go to the OS keychain on save and are never written to the config file. The
config records only which credentials exist. `keyring` is behind a trait so tests can
substitute an in-memory store without touching the user's login keychain.

Slack's cookie must be pasted exactly as it appears in the browser's `Cookie:` header,
percent-escapes intact. The field says so, because decoding it is the single most likely
way to break an otherwise-correct setup, and the failure looks like a wrong token rather
than a mangled cookie.

**`auth.test` is the one Slack call worth keeping.** It is cheap, it exercises both
credentials together, and it is the call whose failure message the user will actually be
reading at 6pm when the cookie has expired.