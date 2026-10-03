# Evidence is a bounded text rendering, not a structured object

**Status:** Accepted.

Each Fire produces one `Evidence` value: a single string of rendered text plus counts,
handed to the model as a single block. It is not a structured document the model parses,
and it carries no field names beyond the section headings.

## Why

The model writes prose for a human audience. A schema it must fill in pushes it toward
enumerating rather than writing, and a half-filled schema produces a draft that reads
like a form filled in badly.

Trimming has to happen before the model sees anything, because the budget is known in
tokens and not in fields. A string trims cleanly; a structure would need a second,
parallel notion of which parts were cut.

## Alternatives considered

### A structured evidence object (JSON of activities)
- Pros: structured; a downstream consumer could filter it.
- Cons: pushes the model toward enumerating rather than writing, and a half-filled
  structure produces a draft that reads like a bad form. There is no consumer.
- Rejected: the model's output is prose for a human; the input should be shaped like it.

### Structured, with the schema enforced
- Pros: guarantees well-formed input.
- Cons: the same pressure, plus the model spends its budget describing structure.

### Trimming after rendering
- Pros: the structure is complete before it is cut.
- Cons: the budget is known in tokens, not in fields, so a structure needs a second
  parallel notion of what was cut.
- Rejected: trim the string before the model sees it; it trims cleanly and the counts
  still say what was dropped.

## Consequences

Every count in the rendered text must be **true of what survived the trim**, not of what
was fetched. A Draft built from 15 messages reads as a 15-message day. The alternative —
saying "showing 15 of 200" and letting the model decide how much that matters — invites
it to write a confident summary of a fraction.

The renderer's last line is a coverage statement the model is told to respect:
`## Coverage: 15 of 200 messages, 2 of 2 commits`. Truncation is therefore visible in
the Evidence text itself, not only in the UI, so the model can refuse to overstate.

Structure still exists behind the boundary: `Activity` values, `ActivitySource`
implementations and the renderer are ordinary Rust types. The decision is only that none
of that structure crosses into the prompt.

Source order within Evidence is by time across both Slack and GitHub, because a status
post is about a day, not about two integrations. Grouped-by-source was considered and
rejected: it invites the model to write one paragraph per integration, which is the
failure mode this tool exists to avoid.