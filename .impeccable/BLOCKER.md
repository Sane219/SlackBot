# Impeccable: build state and known gaps

## Build path taken: code-led, not comp-led

`.impeccable/config.json` records `"buildPath": "comp"`. That was the recorded
preference, and it was not what happened. This file states what did.

**The comp round cannot run on this machine.** Three independent blockers:

1. `OPENAI_API_KEY` is unset, so `impeccable generate-image` refuses.
2. No harness-native image tool is available.
3. `serve-question` is absent, so the decision page cannot draw wireframe schematics
   either.

Comp-first needs at least one of those. With none, the build proceeded **code-led**:
the direction contract in `.impeccable/surfaces/` carried the ambition, and the finish
work audited the render against that contract rather than against measured pixels.

**No image was fabricated to pass the `comps` gate.** `.impeccable/mocks/` holds three
composition records as `.json` sidecars. They are specifications, not renders.

To make a future surface comp-led: set `OPENAI_API_KEY`, then run
`impeccable build-phase advance`.

## Verdict on the world: code-led build, and it holds up

The direction was the incident postmortem wall — seed `1d97c764`, chosen by the roll,
then pushed bolder by the user into the wall's gap discipline. Rendered in headless
Chrome at 1440×900 and 390×844 and inspected on every tab, the built world matches the
contract:

- One type size, rank by weight and rule
- One amber, one vermilion, used once each
- No shadows except the failure inset; one radius, on the hatch
- A gap drawn as a hatched void with its span printed
- WCAG AA measured, not asserted: every ink token clears 4.5:1

`DESIGN.md` and `.impeccable/design.json` now record the world from the **built CSS**,
verified token by token. `impeccable doctor` reports no drift.

## What rendering found that reading the code did not

Recorded here because the pattern matters more than the individual bugs:

- **Every view rendered at once.** A `display: flex` rule beat the `hidden` attribute's
  UA `display: none`. A DOM probe checking `el.hidden` reported the state as correct —
  the property was right and the rendering was wrong. Only a screenshot found it.
- A Fire's error reason clipped mid-word in the 180px spine.
- Save and Verify wrapped under the field; the action column was too narrow.
- The window label showed a Draft's window on the Setup board.
- `favicon.ico` 404'd on every load.

## Detector: one advisory deliberately kept

`impeccable detect` reports `repeating-stripes-gradient` on the hatch fill. That
gradient **is** the gap — the single load-bearing idea in this direction. Suppressing
it would remove the feature. Kept intentionally.

Two findings were adjudicated rather than obeyed:

- **Contrast was a straight bug, and fixed over the aesthetic.** `--carbon-faint` and
  `--signal-dim` measured 3.1:1 and 3.3:1. The committed AA bar outranked the palette.
- **The `side-tab` finding was a false positive against this world.** Three accent tabs
  *were* drifting toward the generic pattern, so they became one continuous strip down
  the spine. `border-left: 3px` no longer appears anywhere.

## BLOCKER.md — why this file exists

## The comp round was unavailable, not skipped

The visual system was worked without a rendered comparison. The environment had no
`OPENAI_API_KEY`, no image tool, and no `serve-question` verb in the installed v4.2.0
binary. Nothing was substituted to fake one. The system in `DESIGN.md` was written by
hand from the built CSS and then verified token by token against it.

**This is stale in one respect:** the installed engine now reports v4.5.0 skill files, so
the missing verbs may be available. Re-running a comp round would be the way to close this
properly.

## The hatch's repeating gradient *is* the gap, deliberately

One detector finding is accepted rather than fixed: the hatch uses a `repeating-linear-gradient`
where a solid fill would do. There is no hand-drawn alternative — a gap is drawn as a gap,
and the stripes are the drawing. `--rule` at 1px on `--grid-soft` is indistinguishable from
the rules around it, which is the point: the void has to read as void, not as a component.

## A stale reference in the sidecar

`.impeccable/design.json` was generated from the pre-React UI. Its 6 component snippets
were checked against the current `ui/src/styles.css` and every class in them still exists
(the classes were carried across the migration), so it is usable as-is. It names no file
paths, so it did not rot. If a future UI change removes one of those classes, the sidecar
needs regenerating rather than hand-editing.

## Known gaps

- `impeccable detect` has not been run against the React components. It was not on `PATH`
  when the UI was rewritten.
- No Windows CI job, so the cross-platform claim is checked once by hand rather than
  continuously (`cargo check --target x86_64-pc-windows-msvc` passes as far as the bundled
  SQLite C build, which needs Visual Studio Build Tools).
- No real end-to-end run: every Fire to date has been driven by mocks or a seeded database.
  No genuine Slack or GitHub credentials have ever driven a Fire through auto-send.

Not fixed, and recorded rather than assumed away:

- **Repo discovery is unthrottled.** The hour-cache charting asked for was never
  implemented, because the research showed it targeted the wrong cost.
- **Slack channel discovery reads one channel.** ADR-0003's "discovered from activity"
  holds for GitHub repos and not for Slack channels.
- **No comp means no measured target.** The layout is audited against the direction
  contract, not against pixels. The 180px spine was chosen by judgement and checked by
  screenshot, not measured against a comp.
