# A Draft stores its text, its window, its counts and its parent — nothing more

**Status:** Accepted.

`Draft` holds the rendered text, the Context Window it was drawn from, the source counts
that rendering reported, the Job and Fire that produced it, and a nullable reference to
the Draft it was seeded from. The Evidence text itself is not stored, per the standing
decision that company content stays off disk.

## Why

The parent reference exists because the morning post is seeded from the previous
cycle's Draft rather than from its own window's activity. Without lineage, a chain of
three posts has no recorded relationship and cannot be explained to the user.

Seeding prefers the **last edited** text, not the last generated text. A user who rewrote
yesterday's summary has already decided what matters, and re-deriving from raw evidence
would discard that judgement. A discarded Draft is removed from the chain and the
grandparent becomes the seed.

## Alternatives considered

### Store the raw evidence alongside the Draft
- Pros: a Draft could be explained or re-derived exactly.
- Cons: company message content on disk, in a tool that is otherwise careful about this.
  ADR-0002 established that credentials never touch a file; content is a larger leak than
  a token because it is readable, and it is duplicated in Slack anyway.
- Rejected: keep counts and the window, discard the source. Re-`Regenerate` re-fetches.

### Store only the Draft text, no window or counts
- Pros: the smallest schema.
- Cons: a Draft cannot be explained, and Regenerate cannot re-fetch the *original* window
  without knowing it — the text could drift to mean something else by pressing a button
  on Thursday.
- Rejected: the window is not evidence, it is provenance, and it is small.

### Seed from the last *generated* Draft
- Pros: simpler; seeding ignores edits.
- Cons: a user who rewrote yesterday's summary has already decided what matters, and
  re-deriving from raw evidence discards that judgement.
- Rejected: prefer the last *edited* text.

## Consequences

Regenerate cannot be free. Evidence is not persisted, so re-rendering means re-fetching
the original Context Window — which may be hours old, whose Slack messages may since have
been edited, and whose GitHub `since` window will now match the same span against newer
`updated_at` values. Regenerate therefore re-fetches the **original window**, never the
current one, so a Draft's meaning does not drift just because the user pressed a button
late. The UI states that re-fetching can differ from the first render.

Counts are stored because the UI must be able to say what a Draft was drawn from without
keeping the source. That is the whole of what `count_json` is: a summary for display, not
a cache.

Deleting a Draft is allowed and orphans its children; the chain resolves to the nearest
surviving ancestor. Cascading deletes would destroy history a user might want to read.