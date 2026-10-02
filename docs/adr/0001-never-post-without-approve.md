# Never post without an explicit Approve

No code path in this tool sends a message to Slack without a human clicking Approve on a
specific Draft. Not on a timer, not when confidence is high, not when the user has
approved a previous one, not after an outage drains the Inbox.

## Why

This tool drafts a colleague's professional status into a channel their team reads. A
wrong post is a credibility cost borne by a human who did not choose to bear it, in a
medium that cannot be quietly edited after the fact. Drafting is reversible and
recoverable; posting is neither.

The cost is real and accepted: three clicks a day that a fully-automatic version would
not need, and a Draft that sits unapproved is a Draft the user still has to notice.

## Consequences

`chat.postMessage` is reachable from exactly one place in the codebase — the Approve
handler. Every other route reads or drafts. This is the property the rest of the design
leans on, and it is worth a test that fails if a second caller ever appears.

A missed Fire produces a visible gap in the Inbox rather than a silent non-event. Silence
is what makes an unattended tool untrustworthy; a gap the user can see is merely
inconvenient.