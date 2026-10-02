// Jobs: the planner, the form, and "run it now".
//
// A Job is never gated on the model. The planner proposes, every field is editable, and
// the by-hand form always works — if the Plan Role fails or the description is vague,
// you are not stuck.

import { useEffect, useState } from "react";
import { api, tzLabel, windowLabel } from "./api.js";

function ChannelPicker({ value, onChange, channels, loading }) {
  return (
    <select
      className="field"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      aria-label="Channel"
      disabled={loading}
    >
      <option value="">{loading ? "loading channels…" : "choose a channel"}</option>
      {channels.map((c) => (
        <option key={c.id} value={c.name}>
          #{c.name}
        </option>
      ))}
    </select>
  );
}

const EMPTY = {
  name: "",
  at: "09:30",
  tz: Intl.DateTimeFormat().resolvedOptions().timeZone || "Asia/Kolkata",
  channel: "",
  context: "lookback",
  since_at: "18:30",
  previous_day: true,
  lookback_hours: 8,
  prompt: "",
};

function toPayload(job, channelName) {
  return {
    name: job.name,
    at: job.at,
    tz: job.tz,
    channel_id: channelName,
    channel_name: channelName,
    context: job.context,
    since_at: job.since_at,
    previous_day: job.previous_day,
    lookback_hours: job.lookback_hours,
    prompt: job.prompt,
  };
}

function Planner({ channels, onSaved, notify }) {
  const [description, setDescription] = useState("");
  const [proposals, setProposals] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);

  const ask = async () => {
    setBusy(true);
    setError(null);
    try {
      const result = await api.plan(description);
      // Every proposal is editable, and its channel starts as the name the model gave.
      setProposals(
        result.jobs.map((j) => ({
          name: j.name,
          at: j.at,
          tz: j.tz,
          channel: j.channel.replace(/^#/, ""),
          context: j.context,
          since_at: j.since_at || "18:30",
          previous_day: Boolean(j.previous_day),
          lookback_hours: 8,
          prompt: j.prompt,
        })),
      );
    } catch (err) {
      // An unusable proposal is shown raw rather than repaired into something the user
      // did not choose.
      setError(err.message);
    }
    setBusy(false);
  };

  const saveAll = async () => {
    setBusy(true);
    setError(null);
    const failed = [];
    let saved = 0;
    for (const job of proposals) {
      if (!job.name.trim() || !job.channel) {
        failed.push(`${job.name || "unnamed"} (needs a name and a channel)`);
        continue;
      }
      try {
        await api.createJob(toPayload(job, job.channel));
        saved += 1;
      } catch (err) {
        failed.push(`${job.name}: ${err.message}`);
      }
    }
    setProposals(failed.length ? null : null);
    setError(failed.length ? `${saved} saved, ${failed.length} not: ${failed.join("; ")}` : null);
    if (saved > 0) onSaved();
    setBusy(false);
  };

  return (
    <div className="plan">
      <h2 className="step-group__title">Describe your routine</h2>
      <p className="step__note">
        In your own words: what you post, how often, and what each one covers. The model
        proposes; you confirm before anything is scheduled.
      </p>
      <textarea
        rows={3}
        value={description}
        placeholder="I post a day task at 9:30, a progress update around 2:30, and a day summary before I leave. All in #coot-ai. Times are IST."
        onChange={(e) => setDescription(e.target.value)}
        aria-label="Describe your posting routine"
      />

      <div className="actions">
        <button
          className="btn btn--approve"
          disabled={busy || !description.trim()}
          onClick={ask}
        >
          {busy ? "asking…" : "Propose jobs"}
        </button>
      </div>

      {error && (
        <div className="step__error" role="alert">
          {error}
        </div>
      )}

      {proposals && (
        <div className="proposals">
          {proposals.map((job, i) => (
            <div className="proposal" key={i}>
              <div className="proposal__grid">
                <label>
                  <span>Name</span>
                  <input
                    className="field"
                    value={job.name}
                    onChange={(e) =>
                      setProposals((ps) => ps.map((p, j) => (j === i ? { ...p, name: e.target.value } : p)))
                    }
                  />
                </label>
                <label>
                  <span>Time</span>
                  <input
                    className="field field--time"
                    value={job.at}
                    placeholder="HH:MM"
                    onChange={(e) =>
                      setProposals((ps) => ps.map((p, j) => (j === i ? { ...p, at: e.target.value } : p)))
                    }
                  />
                </label>
                <label>
                  <span>Timezone</span>
                  <input
                    className="field"
                    value={job.tz}
                    onChange={(e) =>
                      setProposals((ps) => ps.map((p, j) => (j === i ? { ...p, tz: e.target.value } : p)))
                    }
                  />
                </label>
                <label>
                  <span>Channel</span>
                  <ChannelPicker
                    value={job.channel}
                    channels={channels}
                    onChange={(v) =>
                      setProposals((ps) => ps.map((p, j) => (j === i ? { ...p, channel: v } : p)))
                    }
                  />
                </label>
                <label>
                  <span>Reads</span>
                  <select
                    className="field"
                    value={job.context}
                    onChange={(e) =>
                      setProposals((ps) => ps.map((p, j) => (j === i ? { ...p, context: e.target.value } : p)))
                    }
                  >
                    <option value="lookback">the last N hours</option>
                    <option value="since">since a wall-clock time</option>
                  </select>
                </label>
                <label className="proposal__wide">
                  <span>Instructions for writing this post</span>
                  <textarea
                    rows={2}
                    className="field"
                    value={job.prompt}
                    onChange={(e) =>
                      setProposals((ps) => ps.map((p, j) => (j === i ? { ...p, prompt: e.target.value } : p)))
                    }
                  />
                </label>
              </div>
              <div className="proposal__hint">
                {job.context === "since"
                  ? `reads since ${job.since_at}${job.previous_day ? " the previous day" : ""}`
                  : `reads the last ${job.lookback_hours} hours`}
              </div>
            </div>
          ))}
          <div className="actions">
            <button className="btn btn--approve" disabled={busy} onClick={saveAll}>
              {busy ? "saving…" : "Save all"}
            </button>
            <button
              className="btn btn--secondary"
              disabled={busy}
              onClick={() => setProposals(null)}
            >
              Discard these
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function AddByHand({ channels, onSaved, notify }) {
  const [open, setOpen] = useState(false);
  const [job, setJob] = useState(EMPTY);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const set = (k) => (e) => setJob((j) => ({ ...j, [k]: e.target.value }));

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.createJob(toPayload({ ...job, lookback_hours: Number(job.lookback_hours) }, job.channel));
      setJob(EMPTY);
      setOpen(false);
      onSaved();
    } catch (err) {
      setError(err.message);
    }
    setBusy(false);
  };

  if (!open) {
    return (
      <div className="actions">
        <button className="btn btn--secondary" onClick={() => setOpen(true)}>
          + add one by hand
        </button>
      </div>
    );
  }

  return (
    <div className="manual">
      <h3 className="step-group__title">Add a Job</h3>
      <div className="proposal__grid">
        <label>
          <span>Name</span>
          <input className="field" value={job.name} onChange={set("name")} placeholder="Day Task" />
        </label>
        <label>
          <span>Time</span>
          <input className="field field--time" value={job.at} onChange={set("at")} placeholder="09:30" />
        </label>
        <label>
          <span>Timezone</span>
          <input className="field" value={job.tz} onChange={set("tz")} />
        </label>
        <label>
          <span>Channel</span>
          <ChannelPicker value={job.channel} channels={channels} onChange={set("channel")} />
        </label>
        <label>
          <span>Reads</span>
          <select className="field" value={job.context} onChange={set("context")}>
            <option value="lookback">the last N hours</option>
            <option value="since">since a wall-clock time</option>
          </select>
        </label>
        <label>
          <span>{job.context === "since" ? "Since (HH:MM)" : "Lookback (hours)"}</span>
          <input
            className="field field--time"
            value={job.context === "since" ? job.since_at : job.lookback_hours}
            onChange={job.context === "since" ? set("since_at") : set("lookback_hours")}
          />
        </label>
        <label className="proposal__wide">
          <span>Instructions for writing this post</span>
          <textarea
            rows={2}
            className="field"
            value={job.prompt}
            onChange={set("prompt")}
            placeholder="Three bullets, each *Title*: explanation, in our house style."
          />
        </label>
      </div>
      {job.context === "since" && (
        <label className="checkline">
          <input type="checkbox" checked={job.previous_day} onChange={(e) => setJob((j) => ({ ...j, previous_day: e.target.checked }))} />
          <span>that time was yesterday</span>
        </label>
      )}
      {error && <div className="step__error" role="alert">{error}</div>}
      <div className="actions">
        <button className="btn btn--approve" disabled={busy} onClick={save}>
          {busy ? "saving…" : "Save job"}
        </button>
        <button className="btn btn--secondary" disabled={busy} onClick={() => setOpen(false)}>
          Cancel
        </button>
      </div>
    </div>
  );
}

function JobRow({ job, channels, onDone, onGoToInbox, notify }) {
  const [busy, setBusy] = useState(null);
  const [error, setError] = useState(null);

  const run = async (name, fn) => {
    setBusy(name);
    setError(null);
    try {
      await fn();
      onDone();
    } catch (err) {
      setError(err.message);
    }
    setBusy(null);
  };

  return (
    <div className="step job">
      <div className="step__code">{job.enabled ? "ON" : "OFF"}</div>
      <div className="step__body">
        <div className="step__label">{job.name}</div>
        <div className="step__note">
          {job.schedule.at} {tzLabel(job.schedule.tz)} · {windowLabel(job.context_window)} · #
          {job.channel.name}
        </div>
        {error && <div className="step__error">{error}</div>}
      </div>
      <div className="step__actions">
        {/* Discoverable on purpose: the first real Draft should arrive in seconds, not
            tomorrow at 09:30. */}
        <button
          className="btn btn--secondary"
          disabled={busy !== null}
          onClick={() => run("fire", async () => { await api.fireJob(job.id); onGoToInbox(); })}
        >
          {busy === "fire" ? "running…" : "Run it now"}
        </button>
        <button
          className="btn btn--secondary"
          disabled={busy !== null}
          onClick={() => run("toggle", () => api.updateJob(job.id, { enabled: !job.enabled }))}
        >
          {job.enabled ? "Disable" : "Enable"}
        </button>
      </div>
    </div>
  );
}

export function Jobs({ jobs, channels, channelsLoading, onDone, onGoToInbox }) {
  return (
    <>
      {jobs.length === 0 ? (
        <div className="empty">
          No Jobs yet. Describe your routine below, or add one by hand — the planner is a
          shortcut, never a requirement.
        </div>
      ) : (
        <div className="jobs">
          {jobs.map((job) => (
            <JobRow
              key={job.id}
              job={job}
              channels={channels}
              onDone={onDone}
              onGoToInbox={onGoToInbox}
            />
          ))}
        </div>
      )}

      <Planner channels={channels} onSaved={onDone} />
      <AddByHand channels={channels} onSaved={onDone} />
    </>
  );
}