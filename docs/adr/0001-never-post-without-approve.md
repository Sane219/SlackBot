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

That test has to be written carefully, and the obvious version was worthless: it skipped
any source file whose text mentioned `chat.postMessage`, which is the HTTP layer — the
only file with a real caller. It now matches on the call signature across every module and
asserts the call sits inside `approve` specifically, so moving it to another handler fails.

**Loopback is not an authorisation boundary.** Any page in any browser can reach
`127.0.0.1`, so a hostile page could POST to Approve with no token and no human click and
post to a team channel. Writes therefore require same-origin (`Sec-Fetch-Site`, falling
back to `Origin`) and a JSON content type, which a cross-origin form post cannot send
without a preflight. Verified live: the bypass returned 200 before and 403 after.

**Approve is atomic.** The Draft is claimed before the Slack call and released if the send
fails. Reading `approved` and *then* posting was not safe: two concurrent requests both saw
false and both posted, and a failed send silently consumed a Draft the user never sent.

**A failure is never a Draft.** A model error must not become Draft text, because a user
could then approve and post `DRAFT FAILED: error sending request for http://127.0.0.1:0/…`
to their team. A failed Draft is empty, and Approve refuses an empty Draft.

A missed Fire produces a visible gap in the Inbox rather than a silent non-event. Silence
is what makes an unattended tool untrustworthy; a gap the user can see is merely
inconvenient.