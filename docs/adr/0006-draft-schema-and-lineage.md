# A Draft stores its text, its window, its counts and its parent — nothing more

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