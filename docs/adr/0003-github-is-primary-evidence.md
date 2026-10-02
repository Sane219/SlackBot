# GitHub is the primary evidence source, and local git is the exception

Each Fire collects what the person did in GitHub — comments they wrote, PRs they
authored or reviewed, issues they moved, label and status changes they made — and falls
back to local `git log` only for repositories that happen to be checked out. Which
repositories matter is discovered from recent activity rather than configured.

## Why

The people who most need this tool are not the ones who write the code. Project managers
and manual testers produce their work as comments, reviews and board moves, not commits.
A design that treats `git log` as the main source and GitHub as a later addition is a
tool for engineers, and it would show a tester an empty day.

Discovering repositories from activity rather than configuring them follows from the
same fact: a PM's set of relevant repositories changes when they are reassigned, which
is often and without announcement. A pinned list goes stale and then silently collects
nothing.

## Consequences

A GitHub token is required, not optional. The anonymous API allows 60 requests per hour,
which a handful of users across a few repositories will exhaust. Slack no longer has
exclusive claim on the "one token per external service" story in setup.

The Evidence renderer cannot assume a person has commits. A Fire's evidence may be
entirely comments, and a Day Summary built from nothing but PR reviews is a valid day.
The Day Task Post, which describes intentions, has the weakest evidence of the three and
must be willing to say nothing rather than invent something.

Local `git log` is retained rather than dropped: it is free, it needs no token, and it
gives a commit subject and touched-file list faster than any API round trip.