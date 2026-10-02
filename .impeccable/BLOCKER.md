# Comp round blocked: no image generator on this machine

`impeccable build-phase advance` fails the `comps` gate because it counts image files
under `.impeccable/mocks/`. There is no way to produce them here:

- `OPENAI_API_KEY` is unset, so `impeccable generate-image` refuses.
- This harness exposes no native image tool.
- v4.2.0 has no `serve-question` verb, so the decision page cannot draw wireframes either.

Three composition records exist under `.impeccable/mocks/` as `.json` sidecars, with
composition A carrying `"approved": true`. They are **specifications, not renders.** No
image was fabricated to satisfy the gate.

The compositions were put to the user through the structured question tool, which is the
documented fallback approval mechanism. Approval is genuine and recorded.

Consequence: the build proceeds **code-first** for this surface rather than comp-led. The
ambiguity that a rendered comp would have resolved — exact column proportions and the
density of the timeline spine — is carried in the surface brief's FIRST VIEWPORT block
instead, and the finish review audits the build against that contract rather than
against a measured pixel target.

To make this surface comp-led later: set `OPENAI_API_KEY`, then re-run
`impeccable build-phase advance`.

## Detector: one advisory deliberately kept

`impeccable detect` reports `repeating-stripes-gradient` on the hatch fill. That
gradient is the gap itself — the single load-bearing idea in this direction. Suppressing
it would remove the feature. Kept intentionally.

Contrast was the one finding that was a straight bug: `--carbon-faint` and
`--signal-dim` measured 3.1:1 and 3.3:1. All text now clears WCAG AA, worst case
5.07:1. The committed accessibility bar outranked the palette.

The `side-tab` finding was adjudicated, not obeyed: a 3px status rule per card reads as
the generic AI card pattern. Replaced with one continuous strip down the spine, which
reads as a chart trace instead. `border-left: 3px` no longer appears anywhere.
