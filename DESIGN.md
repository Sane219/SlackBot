---
name: SlackBot
description: The operations board for a working day
colors:
  wall: "#0a0b0d"
  wall-raised: "#101216"
  grid: "#1b3a3f"
  grid-soft: "#14282c"
  carbon: "#e6eaef"
  carbon-dim: "#a8b1ba"
  carbon-faint: "#7d868f"
  signal: "#ffb020"
  signal-dim: "#c2830f"
  sev1: "#ff5f57"
typography:
  body:
    fontFamily: "Roboto Condensed, Archivo Narrow, Helvetica Neue, Arial, sans-serif"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontSize: "0.78em"
    letterSpacing: "0.16em"
    textTransform: "uppercase"
rounded:
  hatch: "1px"
spacing:
  xs: "0.45rem"
  sm: "0.75rem"
  md: "1rem"
  lg: "1.5rem"
  xl: "2.25rem"
components:
  button-approve:
    backgroundColor: "transparent"
    textColor: "{colors.signal}"
    rounded: "{rounded.hatch}"
    padding: "0.6rem 1.4rem"
  button-approve-hover:
    backgroundColor: "{colors.signal}"
    textColor: "{colors.wall}"
  button-secondary:
    backgroundColor: "transparent"
    textColor: "{colors.carbon-dim}"
    rounded: "{rounded.hatch}"
    padding: "0.6rem 1.4rem"
---

# Design System: SlackBot

## Overview

**Creative North Star: "The Operations Board"**

The instrument you read to know what a system did overnight. A board that could shout
would be a board you could no longer trust, so this one takes up no attention at all:
one type size, no radius except a single pixel on a hatch, no shadow except one inset
alarm, and one amber mark whose rarity is the entire reason it is legible. Restraint is
the mechanism here, not the mood.

A drawn gap means more than a filled one. When a window collected nothing, the surface
shows a hatched void with the window's span printed across it, reading `NO SIGNAL`. That
single gesture is the system's whole argument: a tool that cannot distinguish *nothing
happened* from *we did not look* is a tool whose confident drafts mean nothing.

The surface is recorded, not designed. It behaves like a board and takes on no
personality of its own, because the content is another person's work and the only job
here is to present it without editorialising.

**Key Characteristics:**

- One type size. Rank comes from weight, case, and rule — never a second size.
- One amber, used on less than 5% of any screen. Its scarcity is the point.
- Vermilion appears exactly once in the system: `SEV1`, a Fire that failed.
- Depth is a rule and a slightly lighter ground, never a lift.
- A gap is drawn as a gap.

## Colors

Near-black ground with a dim cyan grid, three steps of carbon ink, and two signal
colours. The grid is ruled, not decorative — it is what makes a row a row.

### Primary

- **Signal Amber** (`#ffb020`): the one pen. Marks a Fire that collected evidence, the
  selected tab, the Approve button, and the `NO SIGNAL` label. The only colour a user
  is expected to act on.
- **Signal Amber, quiet** (`#c2830f`): the traced-but-silent tone. Carries the hatch
  fill and any amber that is text rather than a mark, and must still clear 4.5:1.

### Tertiary

- **Alarm Vermilion** (`#ff5f57`): a Fire that failed, and nothing else. It is a
  severity code rendered as a colour.

### Neutral

- **Wall** (`#0a0b0d`): the ground. Near-black with a trace of blue, not pure black.
- **Wall Raised** (`#101216`): a card or a spine. The only depth cue in the system.
- **Grid** (`#1b3a3f`): the strong rule. Divides the spine and marks a selected edge.
- **Grid Soft** (`#14282c`): the ordinary rule. Almost every separation in the UI.
- **Carbon** (`#e6eaef`): body text and anything the user must read.
- **Carbon Dim** (`#a8b1ba`): supporting text — labels, counts, timestamps.
- **Carbon Faint** (`#7d868f`): the code column and placeholder text. The dimmest ink
  that still clears WCAG AA.

### Named Rules

**The One Voice Rule.** Signal Amber is used on less than 5% of any screen. Every
additional amber mark dilutes the ones that matter.

**The Vermilion Rule.** Vermilion is reserved for a failed Fire. If it appears anywhere
else, it has stopped meaning anything and the user will miss the failure that mattered.

**The AA Floor Rule.** Every ink token clears 4.5:1 against `--wall` or
`--wall-raised`. The first draft of this palette failed at 3.1:1 and was raised rather
than shipped — a board you cannot read is not a board.

## Typography

**Display Font:** Roboto Condensed (with Archivo Narrow, Helvetica Neue, Arial)
**Body Font:** Roboto Condensed — the same face. There is no second family.
**Label/Mono Font:** none. Codes are the body face, tracked and uppercased.

**Character:** One condensed grotesque at one size. Condensed because the spine is a
fixed 180px and a wider face would wrap codes into unreadability. Singular because a
second voice would compete with the content, which is someone else's writing.

### Hierarchy

- **Body** (400, 15px, 1.5): everything readable. Draft text, labels, timestamps.
- **Label** (400, `0.78em` ≈ 11.7px, `0.16em` tracking, uppercase): the code column,
  tab labels, classification codes, field captions.
- **Weight instead of size**: section headings are body weight 700. A heading is the
  same size as its content and heavier. Nothing else changes size.

### Named Rules

**The One Size Rule.** A second font size is a system bug. Rank is carried by weight,
case, and rule — the way a ruled board carries rank, not by type scale.

**The 11px Floor.** Labels may shrink to `0.78em` and no further. A code a user has to
zoom to read is a code that has failed at its only job.

## Layout

A fixed two-column board: a **180px spine** at the left edge and a fluid pane to its
right. The spine is the primary axis because a day is a sequence, and a sequence reads
top to bottom. The pane holds one view at a time.

Below **860px** the board collapses to one column, the spine becomes a full-width band
above the pane, and each spine row becomes a three-column strip. The pane's own window
label is hidden there, because it would duplicate the one on the Draft card directly
beneath it.

Spacing is a rem scale from `0.45rem` to `2.5rem`, with `1rem` as the working default.
Nothing is set in raw pixels except the spine width and the two rule weights, both of
which are structural rather than proportional.

**The window is the header.** The Context Window is printed as an edge scale beside the
title, because a status post is only meaningful relative to the span it covers. It
appears on the Inbox only: above the Setup board it described a Draft that had nothing
to do with what was on screen.

## Elevation & Depth

This system has no shadows and does not pretend to. Depth is **tonal layering** — a
raised surface is a slightly lighter ground (`--wall-raised`) — plus a rule. The
container in `src/web/app.css` has `box-shadow: none` throughout.

### Shadow Vocabulary

- **Failure inset** (`box-shadow: inset 0 0 0 1px var(--sev1)`): a vermilion outline
  burned into a failure banner. The only shadow in the system, and it is an alarm rather
  than a lift.

### Named Rules

**The Flat-By-Default Rule.** Surfaces are flat at rest. Nothing rises, nothing glows,
and nothing has a drop shadow. The single exception is the failure inset, which exists
to be the loudest thing on a screen that is otherwise silent.

## Shapes

Square corners everywhere. The only `border-radius` in the entire stylesheet is `1px`,
on the hatch fill that renders a gap — and that is there so the diagonal stripes resolve
rather than alias into a line.

Borders do the work that radius would normally do. A 1px `--grid-soft` rule separates
ordinary content; a 2px `--grid` rule marks a primary division or a selected edge. The
spine's status strip is 3px, the one dimension in the system that is neither a hairline
nor a block.

## Components

### Buttons

- **Shape:** square. No radius, no shadow.
- **Primary (Approve):** transparent ground, `--signal` text and 2px `--grid` border,
  uppercased and tracked at `0.16em`. On hover the ground fills `--signal` and the text
  inverts to `--wall` — the only inversion in the system.
- **Secondary:** transparent ground, `--carbon-dim` text, 2px rule. Hover lifts the text
  to `--carbon` and the border to `--carbon-dim`.
- **Focus:** a 2px `--signal` outline at 2px offset, on every interactive element. It
  exists because WCAG AA requires it, and it is visible on every tab stop.

### Cards / Containers

- **Corner Style:** square.
- **Background:** `--wall-raised`.
- **Shadow Strategy:** none. See Elevation.
- **Border:** 2px `--grid` (`--rule-strong`) on a Draft card; 1px `--grid-soft`
  elsewhere.
- **Internal Padding:** `1.25rem 1.4rem` on a Draft card, `0.9rem 1rem` on a spine row.

### Inputs / Fields

- **Style:** 1px `--grid-soft` stroke, `--wall` ground, square.
- **Focus:** the border shifts to `--signal`. No glow, no ring, no lift.
- **Placeholder:** `--carbon-faint`.
- **Secret fields:** `type="password"`, and once stored the placeholder becomes
  `•••••••• stored` so the field never implies it is empty.

### Navigation

- **Style:** three tabs, uppercase labels at `0.16em` tracking, separated by `1.25rem`.
- **Selected:** `--signal` text with a 2px `--signal` underline.
- **Unselected:** `--carbon-faint`, no underline.
- **Mobile:** the same three tabs, unchanged. There is no hamburger and no drawer; three
  targets do not need one.

### The Spine

The system's signature component and the reason for the whole design. A fixed 180px
column, one row per Fire, each carrying:

- a 3px status strip at the left edge — `--signal` when traced, `--sev1` when failed,
  transparent when neither
- the clock, in `--carbon-dim` with tabular figures
- the Job name, `--carbon` at weight 700
- the classification code in `--carbon-faint` at `0.78em`
- **the hatch**, a 3px `repeating-linear-gradient` in `--signal-dim` at −45°, drawn only
  when a window collected nothing

A gap is a picture *and* a sentence: the hatch is decorative and carries an
`aria-hidden`, while an `sr-only` span beside it states in words that nothing was
collected. A user who cannot see the hatch still learns the same fact.

## Do's and Don'ts

### Do:

- **Do** keep every colour in `:root`. The palette is enforced by having exactly one
  definition site and no literal colour anywhere else.
- **Do** draw a gap. A window that collected nothing gets a hatched void with its span
  printed, never an empty card and never a hedge sentence.
- **Do** name an absent source in the Evidence. A partial Fire says which integration
  failed, because a silent gap is indistinguishable from a quiet day.
- **Do** separate Approve from Discard with a full rule. They are never adjacent.
- **Do** verify contrast by measurement. Every ink token is checked against its actual
  ground, not judged by eye.
- **Do** rank with weight and rule. One size, always.

### Don't:

- **Don't** add a second font size, a rounded corner, or a drop shadow. Each is a system
  bug, and each is how this becomes a card dashboard.
- **Don't** use vermilion for anything but a failed Fire. Its rarity is what makes it
  readable.
- **Don't** use a left accent border as a card's only distinguishing feature. A
  continuous status strip reads as a chart trace; a per-card accent tab reads as
  generated slop.
- **Don't** fill a gap with something plausible. A quiet day produces `NO SIGNAL` and
  nothing else, and the model is told to reply with exactly that when there is nothing
  to report.
- **Don't** let the window label describe a Draft on a screen showing something else.
- **Don't** smooth truncation away. Counts in the Evidence describe what survived the
  trim, and the coverage line says so.
