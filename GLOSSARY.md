# SlackBot

A local tool that reconstructs a person's workday from their own Slack messages and
their work recorded in GitHub, and proposes the recurring status messages their team
expects — but never publishes one on its own.

## The messages

A **Post** is one instance of a recurring status message, addressed to a channel, on one
occasion. Which messages recur, and what each is called, is not fixed here. Teams name
and shape them differently, and this tool learns a team's own from the description its
users give at setup.

What follows describes the kinds of Post that are common. Treat them as vocabulary for
reading a user's description, not as the shapes this tool imposes.

**Day Task**:
A Post made at the start of the working day, naming what the person intends to work on.
It describes intentions, not completions.
_Avoid_: Morning plan, todo list, day plan

**Progress Update**:
A Post made partway through the day, reporting what has landed and what is being asked of
others. It usually ends with a request for review or testing.
_Avoid_: Midday update, checkpoint

**Day Summary**:
A Post made at the end of the day, reporting what was completed and naming a next step.
_Avoid_: Day Log, wrap-up

The tool holds no opinion on these names. If a team says "Day Status", the Job is called
"Day Status" and the Draft is written to match. Hard-coding a name would make the tool
wrong in every workspace except the one it was written in.

## Scheduling

**Job**:
The recurring definition of one Post — when it is due, which channel it goes to, what
evidence it draws on, and the instructions for writing it. A Job is a rule, not a
message. Configuring a Job creates no Post and sends nothing.
_Avoid_: Task, schedule entry, reminder

**Fire**:
One occasion on which a Job came due. A Fire is an event that produces at most one
Draft. It is not a retry, not a retryable unit, and not a Job.
_Avoid_: Tick, trigger, run, execution

**Context Window**:
The span of time before a Fire whose activity counts as evidence for it. A Window can
be a fixed lookback, or a span anchored to a wall-clock time such as "since yesterday
evening" — because a morning Post draws on yesterday, not on the last twelve hours.
_Avoid_: Range, period, lookback

**Schedule**:
When a Job comes due, expressed as a wall-clock time in a named timezone.
_Avoid_: Cron, timer, trigger time

## Evidence

**Activity**:
Something the person did or said within a Context Window, drawn from Slack or Git and
attributed to them by identity.
_Avoid_: Event, log entry, input

**Evidence**:
The Activity within a Context Window, rendered as the text a Post is written from. The
model sees Evidence, never raw history.
_Avoid_: Context, payload, source data

## Producing and sending

**Draft**:
The proposed text of a Post, generated from Evidence. A Draft is inert: it can be
edited, regenerated, or discarded, and doing none of those things changes nothing
outside this tool.
_Avoid_: Suggestion, preview, proposal, generated message

**Inbox**:
The single place unapproved Drafts wait, newest first.
_Avoid_: Queue, review list, outbox

**Approve**:
The deliberate act of sending a Draft to its channel as a Post. Approve is the only
action in this tool that reaches Slack. There is no path that sends without it.
_Avoid_: Post, submit, publish, confirm

## The two model roles

**Plan Role**:
The one-time use of a model at setup, turning a plain-English description of someone's
posting routine into a set of proposed Jobs. Its output is a proposal awaiting
confirmation.
_Avoid_: Setup prompt, config generation, onboarding

**Draft Role**:
The recurring use of a model on each Fire, turning Evidence into a Draft. It has no
authority over schedules and no ability to send.
_Avoid_: Summarizer, generator, writer