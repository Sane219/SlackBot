# UX — the flow this app is actually for

The shape a person walks through, and the decisions behind it. Written because the flow
is the product; the code is just how it renders.

## The thing being optimised

Three times a day, someone decides whether a drafted status post is good enough to send
under their own name. Everything below serves that one moment.

The failure modes this app exists to avoid, in order of how much they cost:

1. Posting something wrong under your own name. Unrecoverable — no recall.
2. Missing a scheduled post, and not noticing until someone asks.
3. Postponing a draft because reading it was work.

## The flow

```
cargo run → browser opens
   │
   ▼
┌─ CHECKLIST ────────────────────────────────────────────┐
│  1  Connect Slack      token + cookie      [VERIFY]     │
│  2  Connect GitHub     token               [VERIFY]     │
│  3  Set the model      endpoint + name + key            │
│                                                          │
│  progress: ▓▓▓░░░  2 of 3                              │
└──────────────────────────────────────────────────────────┘
   │
   ▼  describe your routine in a sentence
┌─ PROPOSED ─────────────────────────────────────────────┐
│  Day Task        09:30  #coot-ai   [editable fields]    │
│  Progress        14:30  #coot-ai   [editable fields]    │
│  Day Summary     18:30  #coot-ai   [editable fields]    │
│                                  [+ add one by hand]    │
│                                          [SAVE ALL]      │
└──────────────────────────────────────────────────────────┘
   │
   ▼  the app is now live. you close the tab.
   │
   │  ... the next day ...
   ▼
┌─ INBOX ──────────────────┬──────────────────────────────┐
│ 14:30 Progress      ACT  │ Progress Update               │
│ 18:30 Day Summary  ACT  │ window 09:00 → 14:30           │
│ NEXT Day Task  09:30     │ MSG 12  PR 4  GIT 1            │
│                          │ ┌────────────────────────────┐ │
│                          │ │ • *Jitter*: landed in #482 │ │
│                          │ └────────────────────────────┘ │
│                          │ ▸ evidence (12 lines)          │
│                          │ [APPROVE & SEND] [EDIT] [DISCARD]│
└──────────────────────────┴───────────────────────────────┘
```

## Decisions, and what each one refuses

**Setup is a checklist, not a form.** Five credential rows plus a textarea on one screen
is a wall, and a wall gets partially completed. Three numbered steps means there is
always an obvious "next", and progress is visible. Each credential sits inside the step
that needs it, so the page never shows four things you can't act on yet.

Slack comes first because it is what fails first in practice, and the `d` cookie is the
single most likely thing to be wrong.

**A Job is never gated on the model.** Today, if the Plan Role fails or your description
is vague, you get nothing — the primary action depends on an LLM call with no fallback.
That is the worst thing in the app. The planner proposes; every field is editable; and
`+ add one by hand` always works. The model helps, it never gates you.

**Channels are chosen by name.** The endpoint that lists your channels exists and is
never called. You pick from a searchable list of what you're actually in; a `C…` id is
never visible unless someone types one deliberately.

**The spine shows the future, not just the past.** A row at the bottom reading `NEXT ·
Progress Update · 14:30` in the quiet tone. An operations board whose whole value is
telling you what happens next, showing only what already happened, is half a board. You
can close the tab knowing when it comes back.

**Evidence is one click away.** Collapsed by default beside each Draft, showing the raw
lines it was written from. It is the only way to tell a good draft from a plausible one,
and it is the honest answer to "where did this come from".

**Failures are banners, not codes.** A dead cookie currently produces a `SEV1` row whose
reason is only readable by looking hard at a narrow column. An expired session gets a
persistent banner across the Inbox naming the cause and linking into setup, because the
user has to *act* on it and it will not fix itself. The banner is a strip above the work,
never a replacement for it: an early version returned early on it and hid all three
waiting Drafts at the moment the user most needed them.

**A persistent problem states itself every time; its fix is offered once.** The banner
never goes away on its own. But the "Reconnect Slack" link inside it disappears once the
user has actually been to Setup, because a floating Setup button at the bottom-left is
always there and repeating the same action in two places trains the reader to skip the
banner entirely. It comes back on a fresh open, which is when it is worth reading.

**"Run it now" is discoverable.** It exists on the Jobs tab and is barely findable. After
setup, the first real Draft should arrive in seconds, not tomorrow at 09:30 — otherwise
you find out the whole path is broken a day late.

## What is deliberately still simple

Three tabs. One type size. No settings page beyond setup. No theming. No accounts.

Every one of those is a thing this app does not need, and each is a thing that would make
it harder to read a gap at a glance.
