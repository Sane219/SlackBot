// The Inbox: the one screen a user opens three times a day.
//
// Every Draft carries its Evidence one click away, because it is the only way to tell a
// good draft from a plausible one. Approve is separated from Discard by a full rule and
// is the only control that reaches Slack.

import { useState } from "react";
import { api, stamp } from "./api.js";

function Counts({ counts }) {
  const c = counts || {};
  const parts = [
    ["MSG", c.slack_messages],
    ["PR", c.github_comments],
  ].filter(([, v]) => v);

  return (
    <div className="counts">
      {parts.map(([code, n]) => (
        <div className="count" key={code}>
          <span className="count__code">{code}</span>
          <span className="count__n">{n}</span>
        </div>
      ))}
      {c.slack_channels > 0 && (
        <div className="count">
          <span className="count__code">CHAN</span>
          <span className="count__n">{c.slack_channels}</span>
        </div>
      )}
      {c.github_repos > 0 && (
        <div className="count">
          <span className="count__code">REPO</span>
          <span className="count__n">{c.github_repos}</span>
        </div>
      )}
      {/* Truncation states itself rather than hiding in the counts. */}
      {(c.slack_messages < c.slack_fetched || c.github_comments < c.github_fetched) && (
        <div className="count grain">
          trimmed: {c.slack_messages}/{c.slack_fetched} msg,{" "}
          {c.github_comments}/{c.github_fetched} comments
        </div>
      )}
    </div>
  );
}

function DraftCard({ draft, tz, onDone }) {
  const [text, setText] = useState(draft.text);
  const [editing, setEditing] = useState(false);
  const [showEvidence, setShowEvidence] = useState(false);
  const [busy, setBusy] = useState(null);
  const [note, setNote] = useState(null);

  const run = async (name, fn) => {
    setBusy(name);
    setNote(null);
    try {
      const result = await fn();
      if (result?.note) setNote(result.note);
      onDone();
    } catch (err) {
      setNote(err.message);
    }
    setBusy(null);
  };

  // A gap has nothing to approve. Approving it would put NO SIGNAL in a channel.
  if (draft.no_signal) {
    return (
      <article className="draft-card draft-card--nosignal">
        <header className="draft-card__head">
          <div className="draft-card__name">{draft.job_name}</div>
          <div className="window">
            {stamp(draft.window_from, tz)} → {stamp(draft.window_to, tz)}
          </div>
        </header>
        <Counts counts={draft.counts} />
        <div className="draft-card__body">NO SIGNAL</div>
        <div className="actions">
          <span className="grain">Nothing was collected, so there is nothing to post.</span>
          <div className="actions__gap" aria-hidden="true" />
          <button
            className="btn btn--secondary"
            disabled={busy !== null}
            onClick={() => run("discard", () => api.discardDraft(draft.id))}
          >
            Discard
          </button>
        </div>
      </article>
    );
  }

  return (
    <article className="draft-card">
      <header className="draft-card__head">
        <div className="draft-card__name">{draft.job_name}</div>
        <div className="window">
          {stamp(draft.window_from, tz)} → {stamp(draft.window_to, tz)}
        </div>
      </header>

      <Counts counts={draft.counts} />

      {draft.partial && (
        <div className="banner" role="status">
          One source could not be read for this window. The draft covers what arrived.
        </div>
      )}

      {editing ? (
        <textarea
          className="draft-card__edit"
          rows={8}
          value={text}
          onChange={(e) => setText(e.target.value)}
          aria-label={`Edit the ${draft.job_name} draft`}
        />
      ) : (
        <div className="draft-card__body">{text}</div>
      )}

      {/* The only way to tell a good draft from a plausible one. */}
      <button
        className="disclosure"
        aria-expanded={showEvidence}
        onClick={() => setShowEvidence((v) => !v)}
      >
        {showEvidence ? "▾" : "▸"} evidence
        {draft.counts?.slack_messages + draft.counts?.github_comments > 0 && (
          <span className="disclosure__n">
            {" "}
            {draft.counts.slack_messages + draft.counts.github_comments} item
            {draft.counts.slack_messages + draft.counts.github_comments === 1 ? "" : "s"}
          </span>
        )}
      </button>

      {showEvidence && <EvidencePanel draft={draft} tz={tz} />}

      {note && <div className="banner" role="alert">{note}</div>}

      <div className="actions">
        <button
          className="btn btn--approve"
          disabled={busy !== null}
          onClick={() => run("approve", () => api.approve(draft.id))}
        >
          {busy === "approve" ? "sending…" : "Approve & send"}
        </button>

        {editing ? (
          <button
            className="btn btn--secondary"
            disabled={busy !== null}
            onClick={() =>
              run("save", async () => {
                await api.editDraft(draft.id, text);
                setEditing(false);
              })
            }
          >
            Save edit
          </button>
        ) : (
          <button className="btn btn--secondary" onClick={() => setEditing(true)}>
            Edit
          </button>
        )}

        <button
          className="btn btn--secondary"
          disabled={busy !== null}
          onClick={() =>
            run("regen", async () => {
              // Re-fetching re-reads the *original* window, so a Draft's meaning cannot
              // drift because the user pressed a button late.
              if (
                !window.confirm(
                  "Re-fetch the original window and draft again? The result may differ if messages have changed since.",
                )
              ) {
                setBusy(null);
                return;
              }
              await api.regenerate(draft.id);
            })
          }
        >
          {busy === "regen" ? "drafting…" : "Regenerate"}
        </button>

        <div className="actions__gap" aria-hidden="true" />

        <button
          className="btn btn--secondary"
          disabled={busy !== null}
          onClick={() => run("discard", () => api.discardDraft(draft.id))}
        >
          Discard
        </button>
      </div>
    </article>
  );
}

/**
 * The Evidence, rendered for a human.
 *
 * The server deliberately does not keep it (ADR-0006: company content stays off
 * disk), so this is reconstructed from the counts and the window. Where the raw
 * lines are not available, it says so rather than implying the day was quiet.
 */
function EvidencePanel({ draft, tz }) {
  const c = draft.counts || {};
  return (
    <div className="evidence">
      <div className="evidence__line">
        window <strong>{stamp(draft.window_from, tz)} → {stamp(draft.window_to, tz)}</strong>
      </div>
      <div className="evidence__line">
        collected <strong>{c.slack_messages ?? 0}</strong> of {c.slack_fetched ?? 0} Slack
        messages across {c.slack_channels ?? 0} channel
        {c.slack_channels === 1 ? "" : "s"}, and{" "}
        <strong>{c.github_comments ?? 0}</strong> of {c.github_fetched ?? 0} GitHub
        comments across {c.github_repos ?? 0} repo
        {c.github_repos === 1 ? "" : "s"}.
      </div>
      <p className="evidence__note">
        The raw messages are not kept on disk — Slack and GitHub content stays off this
        machine. The draft above was written from what was in this window at the time it
        fired.
      </p>
    </div>
  );
}

export function Inbox({
  drafts,
  zones,
  sessionProblem,
  showSetupLink,
  onDone,
  onGoToSetup,
}) {
  return (
    <>
      {/* A problem that needs the user to act sits above the work, never instead of it.
          An earlier version returned early on the banner, which hid all three waiting
          Drafts at exactly the moment the user most needed to see them.

          The banner is permanent because the cause is. Its button is not: once the user
          has been to Setup, the floating Setup button carries the same action and the
          banner stops asking. A banner that repeats a button ten times a day trains the
          reader to skip the whole strip, including the first line. */}
      {sessionProblem && (
        <div className="banner banner--sev1 banner--sticky" role="alert">
          <strong>{sessionProblem}</strong>{" "}
          {showSetupLink && (
            <button className="linkish" onClick={onGoToSetup}>
              Reconnect Slack
            </button>
          )}
        </div>
      )}

      {drafts.length === 0 ? (
        <div className="empty">Nothing waiting. A Draft appears here when a Job fires.</div>
      ) : (
        drafts.map((draft) => (
          <DraftCard
            key={draft.id}
            draft={draft}
            tz={zones.get(draft.job_id)}
            onDone={onDone}
          />
        ))
      )}
    </>
  );
}
