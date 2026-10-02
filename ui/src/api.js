// One place that knows how to talk to the server.
//
// Every write sends JSON, which is half of the origin boundary (the server refuses
// cross-origin requests and non-JSON content types). Nothing here retries a write:
// ADR-0001 means a retry could post twice.

async function request(path, { method = "GET", body } = {}) {
  const res = await fetch(path, {
    method,
    headers: body ? { "Content-Type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
  });

  const text = await res.text();

  if (!res.ok) {
    throw new Error(reason(text) || `HTTP ${res.status}`);
  }

  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    // A 200 that is not JSON is the server's problem, but the user still needs to be
    // told something other than a blank screen.
    throw new Error("the server returned something this page could not read");
  }
}

/**
 * The reason a request failed, in the server's own words.
 *
 * A failed write answers with `text/plain`, not JSON — `(StatusCode, String)` is what
 * `ApiError` is. An earlier version only looked for a JSON array, so every error in the
 * UI read `HTTP 412`, and the carefully worded explanations the server writes for a
 * browser ("slack is not configured yet", "no channel called #foo that you are a member
 * of") were thrown away. Those messages are the whole reason for writing them.
 *
 * JSON is still handled, because a reason can arrive inside an otherwise-JSON response
 * as a tuple, where the second element is the message.
 */
function reason(text) {
  const trimmed = text.trim();
  if (!trimmed) return null;
  if (!trimmed.startsWith("[") && !trimmed.startsWith("{")) return trimmed;

  try {
    const parsed = JSON.parse(trimmed);
    if (Array.isArray(parsed) && parsed.length > 1) return String(parsed[1]);
    if (parsed && typeof parsed === "object") {
      return parsed.message ?? parsed.error ?? null;
    }
    return null;
  } catch {
    // Looks like JSON but is not. Showing the raw text beats showing nothing.
    return trimmed;
  }
}

export const api = {
  health: () => request("/api/health"),
  setup: () => request("/api/setup"),
  inbox: () => request("/api/inbox"),
  jobs: () => request("/api/jobs"),
  channels: () => request("/api/channels"),
  llmSettings: () => request("/api/setup/llm"),

  saveCredential: (kind, value) =>
    request("/api/setup/credential", { method: "POST", body: { kind, value } }),
  verifyCredential: (kind) =>
    request("/api/setup/verify", { method: "POST", body: { kind } }),
  saveLlm: (base_url, model) =>
    request("/api/setup/llm", { method: "POST", body: { base_url, model } }),

  createJob: (job) => request("/api/jobs", { method: "POST", body: job }),
  updateJob: (id, patch) => request(`/api/jobs/${id}`, { method: "PATCH", body: patch }),
  deleteJob: (id) => request(`/api/jobs/${id}`, { method: "DELETE" }),
  fireJob: (id) => request(`/api/jobs/${id}/fire`, { method: "POST", body: {} }),

  plan: (description) => request("/api/plan", { method: "POST", body: { description } }),

  editDraft: (id, text) => request(`/api/drafts/${id}`, { method: "PATCH", body: { text } }),
  discardDraft: (id) => request(`/api/drafts/${id}`, { method: "DELETE" }),
  regenerate: (id) => request(`/api/drafts/${id}/regenerate`, { method: "POST", body: {} }),
  approve: (id) => request(`/api/drafts/${id}/approve`, { method: "POST", body: {} }),
};

// ── formatting ─────────────────────────────────────────────────────────────

/**
 * A time in a named zone, as `HH:MM`.
 *
 * `Date#getHours` is the *machine's* zone. A Job is scheduled in its own zone, and its
 * Context Window is resolved in that zone too (see `ContextWindow::resolve`), so
 * printing it in the machine's zone makes a 09:30 IST Job show a window that does not
 * start at 09:30 — and it changes when the user travels. The Job's zone is the only one
 * that can be right.
 */
export const clock = (iso, tz) => {
  if (!tz) return clockLocal(iso);
  try {
    return new Intl.DateTimeFormat("en-GB", {
      timeZone: tz,
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    }).format(new Date(iso));
  } catch {
    // An unknown zone must not blank the time. Falling back to the machine's zone shows
    // a time that is arguably wrong; showing nothing shows the user nothing at all.
    return clockLocal(iso);
  }
};

const clockLocal = (iso) => {
  const d = new Date(iso);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
};

/** `2 Oct 14:30`, in the Job's zone when one is given. */
export const stamp = (iso, tz) => {
  const d = new Date(iso);
  const day = d.getDate();
  const month = d.toLocaleString("en", { month: "short" });
  return `${day} ${month} ${clock(iso, tz)}`;
};

/** A map of job id to that Job's timezone, for the places that only hold a `job_id`. */
export function zonesByJob(jobs) {
  return new Map(
    jobs.map((j) => [j.id, j.schedule?.tz]).filter(([, tz]) => Boolean(tz)),
  );
}

// "America/Los_Angeles" -> "Los Angeles"; "UTC" and "GMT" have no slash and must not
// read as "undefined", and an underscore is a filename convention, not English.
export const tzLabel = (tz) =>
  tz && tz.includes("/") ? tz.split("/")[1].replace(/_/g, " ") : tz;

export const windowLabel = (w) =>
  w?.kind === "lookback" ? `last ${w.hours}h` : w?.kind === "since"
    ? `since ${w.at}${w.previous_day ? " prev day" : ""}`
    : `${w?.from ?? ""} → ${w?.to ?? ""}`;

export const OUTCOME_CODE = {
  drafted: "ACT",
  partial: "PR",
  failed: "SEV1",
  missed: "MIS",
  skipped: "SKP",
};
