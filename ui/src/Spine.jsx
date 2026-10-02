// The spine: one row per Fire, plus a NEXT row for what is coming.
//
// This is the system's signature component. A row's status is carried by a 3px strip at
// its left edge, and a gap is a hatched void — a picture the eye reads, with an sr-only
// sentence beside it for anyone who cannot see it.

import { clock, OUTCOME_CODE } from "./api.js";

/**
 * One spine row.
 *
 * The prop is `at`, not `clock`. An earlier version destructured `{ clock: at }` while
 * every caller passed `at`, so the time silently rendered as nothing on every row — the
 * one column the whole spine exists for.
 */
function Row({ at, name, code, tone, hatch, spoken }) {
  const classes = ["fire"];
  if (tone === "failed") classes.push("fire--failed");
  else if (tone === "quiet") classes.push("fire--nosignal");
  else if (tone === "next") classes.push("fire--next");
  else if (tone === "traced") classes.push("fire--traced");

  return (
    <div className={classes.join(" ")}>
      <span className="fire__rule" aria-hidden="true" />
      {at && <div className="fire__clock">{at}</div>}
      <div className="fire__name">{name}</div>
      <div className="fire__code">{code}</div>
      {hatch && <div className="hatch" aria-hidden="true" />}
      {spoken && <span className="sr-only">{spoken}</span>}
    </div>
  );
}

export function Spine({ fires, zones, next, loading }) {
  return (
    <aside className="spine" aria-label="Schedule">
      <div className="spine__label">Fire</div>

      {loading && fires.length === 0 && (
        <div className="fire">
          <span className="fire__rule" aria-hidden="true" />
          <div className="fire__name">connecting…</div>
          <div className="fire__code">SYS</div>
        </div>
      )}

      {!loading && fires.length === 0 && (
        <Row name="no fires yet" code="SYS" />
      )}

      {fires.map((fire) => {
        const failed = fire.outcome === "failed";
        const missed = fire.outcome === "missed";

        return (
          <Row
            key={fire.id}
            at={clock(fire.fired_at, zones.get(fire.job_id))}
            name={fire.job_name}
            // A code, not a sentence. An earlier version appended the first 48 characters
            // of the error, which wrapped to three ragged lines and cut off mid-word with
            // no ellipsis — it read as a rendering fault. The reason is spoken in full for
            // a screen reader, and the actionable version is the banner across the Inbox.
            code={failed ? "SEV1" : OUTCOME_CODE[fire.outcome] || "SYS"}
            tone={failed ? "failed" : missed || fire.no_signal ? "quiet" : "traced"}
            hatch={missed || fire.no_signal}
            spoken={
              failed && fire.error
                ? `Failed: ${firstLine(fire.error)}`
                : missed
                  ? "The app was not running at this time, so nothing was collected."
                  : fire.no_signal
                    ? "No signal. Nothing was collected in this window."
                    : undefined
            }
          />
        );
      })}

      {/* An operations board that only shows the past is half a board. */}
      {next && (
        <Row
          at={next.at}
          name={next.name}
          code="NEXT"
          tone="next"
          spoken={`Next: ${next.name} at ${next.at}.`}
        />
      )}
    </aside>
  );
}

function firstLine(text) {
  return String(text).split("\n")[0];
}

/** The soonest enabled Job that has not fired for its current slot. */
export function nextFire(jobs, fires) {
  const enabled = jobs.filter((j) => j.enabled);
  if (enabled.length === 0) return null;

  const latest = new Map();
  for (const fire of fires) {
    const t = new Date(fire.fired_at).getTime();
    if (!latest.has(fire.job_id) || t > latest.get(fire.job_id)) {
      latest.set(fire.job_id, t);
    }
  }

  const now = Date.now();
  const candidates = enabled
    .map((job) => {
      const [h, m] = (job.schedule.at || "00:00").split(":").map(Number);
      const firedAt = latest.get(job.id);
      if (firedAt === undefined) return { job, at: nextOccurrence(h, m, now, false) };
      // If the job fired today already, the next one is tomorrow.
      const wasToday = new Date(firedAt).toDateString() === new Date(now).toDateString();
      return { job, at: nextOccurrence(h, m, now, wasToday) };
    })
    .filter((c) => c.at > now)
    .sort((a, b) => a.at - b.at);

  if (candidates.length === 0) return null;
  const { job, at } = candidates[0];
  // The Job's zone, so a 09:30 post reads 09:30 wherever the machine is.
  return { name: job.name, at: clock(new Date(at).toISOString(), job.schedule?.tz) };
}

function nextOccurrence(h, m, nowMs, tomorrow) {
  const d = new Date(nowMs);
  d.setHours(h, m, 0, 0);
  if (tomorrow || d.getTime() <= nowMs) d.setDate(d.getDate() + 1);
  return d.getTime();
}
