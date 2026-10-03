// Setup: a three-step checklist, not a wall.
//
// Five credential rows on one screen is a wall, and walls get partially completed. Three
// numbered steps means there is always an obvious next, and progress is visible.
//
// Slack comes first because it is what fails first in practice, and the `d` cookie is
// the single most likely thing to be wrong.

import { useState } from "react";
import { api } from "./api.js";

function CredentialField({ kind, label, note, present, onDone }) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(null);
  const [mark, setMark] = useState(present ? "ok" : "pending");
  const [error, setError] = useState(null);

  const save = async () => {
    if (!value.trim()) return;
    setBusy("save");
    setError(null);
    try {
      await api.saveCredential(kind, value);
      setValue("");
      setMark("pending");
      onDone();
    } catch (err) {
      setError(err.message);
    }
    setBusy(null);
  };

  const verify = async () => {
    setBusy("verify");
    setError(null);
    try {
      const result = await api.verifyCredential(kind);
      setMark(result.ok ? "ok" : "failed");
      if (!result.ok) setError(result.reason);
      onDone();
    } catch (err) {
      setMark("failed");
      setError(err.message);
    }
    setBusy(null);
  };

  return (
    <div className="step__row">
      <label className="step__label" htmlFor={`cred-${kind}`}>
        {label}
      </label>
      <div className="step__body">
        <input
          id={`cred-${kind}`}
          type="password"
          value={value}
          placeholder={present ? "•••••••• stored" : "paste to store"}
          onChange={(e) => setValue(e.target.value)}
          autoComplete="off"
          spellCheck={false}
        />
        <div className="step__note">{note}</div>
        {error && (
          <div className="step__error" role="alert">
            {error}
          </div>
        )}
      </div>
      <div className="step__actions">
        <span className={`step__mark step__mark--${mark}`}>
          {mark === "ok" ? "OK" : mark === "failed" ? "FAILED" : "PENDING"}
        </span>
        <button className="btn btn--secondary" disabled={busy !== null} onClick={save}>
          {busy === "save" ? "…" : "Save"}
        </button>
        <button className="btn btn--secondary" disabled={busy !== null} onClick={verify}>
          {busy === "verify" ? "…" : "Verify"}
        </button>
      </div>
    </div>
  );
}

function LlmField({ onDone }) {
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState(null);
  const [error, setError] = useState(null);

  const save = async () => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      const result = await api.saveLlm(baseUrl, model);
      if (result.note) setNote(result.note);
      onDone();
    } catch (err) {
      setError(err.message);
    }
    setBusy(false);
  };

  return (
    <div className="step__row">
      <div className="step__label">Endpoint and model</div>
      <div className="step__body">
        <input
          type="text"
          value={baseUrl}
          placeholder="http://127.0.0.1:8000/v1"
          onChange={(e) => setBaseUrl(e.target.value)}
          aria-label="Model endpoint"
        />
        <input
          type="text"
          value={model}
          placeholder="model name"
          onChange={(e) => setModel(e.target.value)}
          aria-label="Model name"
        />
        <div className="step__note">Checked at save, so a bad endpoint fails here rather than at 14:30.</div>
        {error && <div className="step__error" role="alert">{error}</div>}
        {note && <div className="step__note">{note}</div>}
      </div>
      <div className="step__actions">
        <button className="btn btn--secondary" disabled={busy} onClick={save}>
          {busy ? "checking…" : "Save"}
        </button>
      </div>
    </div>
  );
}

/**
 * Post with nobody watching (ADR-0010).
 *
 * Off by default and nothing else in the app can turn it on. The cost is stated in the
 * label rather than a dialog: once this is on, a wrong post goes out under your own name
 * and there is no recall, and no one is watching when it happens.
 *
 * It is a checkbox and one POST. Deliberately not per-Job — see ADR-0010 for why the
 * scope is one switch.
 */
function AutoSend({ on, onDone }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);

  const flip = async (next) => {
    setBusy(true);
    setError(null);
    try {
      await api.autoSend(next);
      onDone();
    } catch (err) {
      setError(err.message);
    }
    setBusy(false);
  };

  return (
    <section className="step-group">
      <h2 className="step-group__title">
        <span className="step-group__n">4</span> Auto-send
      </h2>

      <label className="checkline">
        <input
          type="checkbox"
          checked={on}
          disabled={busy}
          onChange={(e) => flip(e.target.checked)}
        />
        <span>
          Send each Draft the moment it is written, without waiting for you
        </span>
      </label>

      <p className="step__note">
        {on
          ? "On. A Draft posts on its own at its scheduled time. A Draft that collected nothing, or that is missing a source, still waits for you — a broken window is not a status update."
          : "Off. Every Draft waits in the Inbox until you click Approve."}
      </p>

      {error && (
        <div className="step__error" role="alert">
          {error}
        </div>
      )}
    </section>
  );
}

export function Setup({ setup, onDone, onDoneJobs }) {
  const present = setup?.present || {};
  const llm = present.llm;
  const slack = present.slack_token && present.slack_cookie;
  const github = present.github;

  // Three, not four. Auto-send is optional and not part of being configured; counting it
  // would mean the board reads "3 of 4" forever on an install that never wants it.
  const steps = [
    { done: slack, label: "Connect Slack" },
    { done: github, label: "Connect GitHub" },
    { done: llm, label: "Set the model" },
  ];
  const doneCount = steps.filter((s) => s.done).length;

  return (
    <div className="steps">
      <div className="progress" role="status">
        <div className="progress__bar" aria-hidden="true">
          <div className="progress__fill" style={{ width: `${(doneCount / 3) * 100}%` }} />
        </div>
        <div className="progress__label">
          {doneCount} of 3 —{" "}
          {doneCount === 3 ? "ready" : steps.find((s) => !s.done)?.label}
        </div>
      </div>

      <section className="step-group">
        <h2 className="step-group__title">
          <span className="step-group__n">1</span> Connect Slack
        </h2>
        <CredentialField
          kind="slack_token"
          label="Slack token"
          note="The xoxc… token. Stored in the OS keychain, never in a file."
          present={present.slack_token}
          onDone={onDone}
        />
        <CredentialField
          kind="slack_cookie"
          label="Slack d cookie"
          note="Paste the value exactly as the browser sends it, percent-escapes intact. Decoding it breaks auth and the error looks like a wrong token."
          present={present.slack_cookie}
          onDone={onDone}
        />
      </section>

      <section className="step-group">
        <h2 className="step-group__title">
          <span className="step-group__n">2</span> Connect GitHub
        </h2>
        <CredentialField
          kind="github_token"
          label="GitHub token"
          note="A token with read access. Stored in the OS keychain."
          present={present.github}
          onDone={onDone}
        />
      </section>

      <section className="step-group">
        <h2 className="step-group__title">
          <span className="step-group__n">3</span> Set the model
        </h2>
        <CredentialField
          kind="llm_api_key"
          label="Model API key"
          note="Stored in the OS keychain. Never written to a file."
          present={llm}
          onDone={onDone}
        />
        <LlmField onDone={onDone} />
      </section>

      {doneCount === 3 && (
        <p className="step__note step__note--done">
          Everything is connected. Describe your routine on the Jobs tab, or add one Job by
          hand.
        </p>
      )}

      <AutoSend on={Boolean(setup?.auto_send)} onDone={onDone} />
    </div>
  );
}
