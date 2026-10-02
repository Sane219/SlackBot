# Authenticate to Slack with a user session token, not an OAuth app

Reads and writes go through an `xoxc-…` user token paired with the session's `d` cookie,
sent as `Authorization: Bearer` plus `Cookie: d=`. There is no Slack app, no OAuth
install, and no bot identity.

## Why

Slack's supported way to post as a specific human is an OAuth `xoxp-…` user token. It
works, and we deliberately are not using it. Every post would carry a visible
"Sent via a bot app" artifact in a channel our colleagues read, which defeats the
purpose of a post that is supposed to read as human. Creating an app also puts an
app in the company's Slack, which is not ours to add.

The session token has no consent screen, no app in the workspace, and no install step —
just two fields pasted into a setup page.

## Consequences

This path is undocumented, and Slack's terms state that undocumented API behaviour may
change without notice. Accepted, because the failure mode is small: when it breaks, the
user pastes two fields back in.

Two costs are accepted knowingly rather than worked around. First, the `d` cookie dies
when the user logs out of Slack, so `invalid_auth` and `not_authed` must fail loudly with
reconnect instructions and must never degrade to drafting from zero Slack context
silently. Second, `chat.delete` does not work on this path, so there is no recall: a
post that was a mistake stays up, and the user fixes it in Slack.

If the artifact ever becomes unacceptable — colleagues objecting to the missing app
attribution, or an org-wide policy — the supported path is the `Sender` boundary, not a
rewrite. Nothing above it may know which mechanism is in use.