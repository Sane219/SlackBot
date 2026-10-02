// The board.
//
// One shell, three tabs, one spine. The spine is always on screen: an operations board
// whose status column is hidden behind a tab is not showing the state of anything
// (ADR-0008).
//
// State is one object refreshed from the server rather than five independent fetchers,
// because the interesting bug in a tool like this is two panels disagreeing about
// whether something was sent.

import { StrictMode, useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";

import { api, stamp, zonesByJob } from "./api.js";
import { Spine, nextFire } from "./Spine.jsx";
import { Inbox } from "./Inbox.jsx";
import { Jobs } from "./Jobs.jsx";
import { Setup } from "./Setup.jsx";

const TABS = [
  ["inbox", "Inbox"],
  ["jobs", "Jobs"],
  ["setup", "Setup"],
];

/**
 * Is the Slack session dead?
 *
 * A dead `d` cookie is the most likely thing to be wrong, and it does not fix itself.
 * It is only visible as a failed Fire, so without this the user reads a narrow code
 * column and has to guess. A banner that names the cause and stays up is the honest
 * version.
 */
function sessionProblem(fires) {
  const recent = fires.filter((f) => f.outcome === "failed" && f.error);
  for (const fire of recent) {
    if (/auth|token_revoked|account_inactive|not_authed|cookie|expired|login_invalid/i.test(fire.error)) {
      return "Slack session expired — the last Fire collected nothing.";
    }
  }
  return null;
}

/**
 * How many of the three setup steps are done.
 *
 * Slack needs both halves of one session, so it counts as one step or none — a token
 * without its cookie is not half a connection, it is an unusable one.
 */
function doneSteps(setup) {
  const p = setup?.present || {};
  return [p.slack_token && p.slack_cookie, p.github, p.llm].filter(Boolean).length;
}

function App() {
  const [tab, setTab] = useState("inbox");
  const [data, setData] = useState({ drafts: [], fires: [] });
  const [jobs, setJobs] = useState([]);
  const [setup, setSetup] = useState(null);
  const [channels, setChannels] = useState([]);
  const [channelsError, setChannelsError] = useState(null);
  const [inboxCount, setInboxCount] = useState(0);
  const [ready, setReady] = useState(false);
  const [failed, setFailed] = useState(false);

  // Guards a state update after the tab is gone.
  const alive = useRef(true);
  useEffect(() => () => { alive.current = false; }, []);

  const refresh = useCallback(async () => {
    // Reads are safe to repeat and cheap; nothing here can reach Slack.
    const [inbox, jobList, status, count] = await Promise.all([
      api.inbox(),
      api.jobs(),
      api.setup(),
      api.health(),
    ]);
    if (!alive.current) return;
    setData(inbox);
    setJobs(jobList.jobs);
    setSetup(status);
    setInboxCount(count.inbox);
    setFailed(false);
    setReady(true);
  }, []);

  useEffect(() => {
    refresh().catch((err) => alive.current && setFailed(err.message));
    const timer = setInterval(() => {
      // A failed poll must not clear the board: an empty Inbox and an unreachable server
      // look identical otherwise, and one of them means "stop reading".
      refresh().catch(() => {});
    }, 20_000);
    return () => clearInterval(timer);
  }, [refresh]);

  // Channels are fetched once per readiness change, not on every poll: they change only
  // when the user joins one, and the list is a Slack round trip.
  const slackReady = Boolean(setup?.present?.slack_token && setup?.present?.slack_cookie);
  useEffect(() => {
    if (!slackReady) return;
    let cancelled = false;
    api
      .channels()
      .then((list) => !cancelled && (setChannels(list), setChannelsError(null)))
      .catch((err) => !cancelled && setChannelsError(err.message));
    return () => {
      cancelled = true;
    };
  }, [slackReady]);

  const problem = sessionProblem(data.fires);
  // Windows are printed in each Job's own zone, which is the zone its schedule and its
  // Context Window are both defined in.
  const zones = zonesByJob(jobs);
  const upcoming = nextFire(jobs, data.fires);
  const waiting = data.drafts.length;

  // The banner names the problem; the button inside it is the action. Once the user has
  // actually been to Setup, that action is redundant with the floating Setup button, so
  // it stops repeating itself on a banner they read ten times a day. `useState` resets
  // on reload, so a fresh open shows it again — which is when it is worth reading.
  const [usedSetup, setUsedSetup] = useState(false);

  const goToSetup = () => {
    setUsedSetup(true);
    setTab("setup");
  };

  // Setup is unfinished, or something it configures has broken. Either way the floating
  // button takes the pen's colour, because it is the one control that resolves it.
  const setupNeedsAttention = doneSteps(setup) < 3 || Boolean(problem);

  return (
    <div className="board">
      <Spine fires={data.fires} zones={zones} next={upcoming} loading={!ready} />

      <main className="pane">
        <header className="pane__head">
          <h1 className="pane__title">{TABS.find(([id]) => id === tab)[1]}</h1>
          {/* The window is the header (DESIGN.md). With several Drafts waiting there is
              no single window, so this names the newest one — the thing the user is most
              likely to be reading. It used to repeat the tab's count, which is the same
              number twice on one screen. */}
          <div className="window">
            {tab === "inbox"
              ? waiting === 0
                ? "nothing waiting"
                : `${stamp(data.drafts[0].window_from, zones.get(data.drafts[0].job_id))} → ${stamp(
                    data.drafts[0].window_to,
                    zones.get(data.drafts[0].job_id),
                  )}`
              : tab === "setup"
                ? `${doneSteps(setup)} of 3 connected`
                : `${jobs.length} job${jobs.length === 1 ? "" : "s"}`}
          </div>
        </header>

        <nav className="tabs" role="tablist" aria-label="Sections">
          {TABS.map(([id, label]) => (
            <button
              key={id}
              className="tab"
              role="tab"
              id={`tab-${id}`}
              aria-selected={tab === id}
              aria-controls={`view-${id}`}
              tabIndex={tab === id ? 0 : -1}
              onClick={() => setTab(id)}
            >
              {label}
              {id === "inbox" && waiting > 0 && <span className="tab__n">{waiting}</span>}
            </button>
          ))}
        </nav>

        {failed && (
          <div className="banner banner--sticky" role="alert">
            Cannot reach the server: {failed}. Restart it with <code>cargo run</code>.
          </div>
        )}

        <section
          className="view"
          role="tabpanel"
          id="view-inbox"
          aria-labelledby="tab-inbox"
          hidden={tab !== "inbox"}
        >
          <Inbox
            drafts={data.drafts}
            zones={zones}
            sessionProblem={problem}
            onDone={refresh}
            onGoToSetup={goToSetup}
            showSetupLink={!usedSetup}
          />
        </section>

        <section
          className="view"
          role="tabpanel"
          id="view-jobs"
          aria-labelledby="tab-jobs"
          hidden={tab !== "jobs"}
        >
          {channelsError && (
            <div className="banner" role="alert">
              Channels could not be loaded: {channelsError}
              {slackReady ? "" : " Connect Slack on the Setup tab first."}
            </div>
          )}
          <Jobs
            jobs={jobs}
            channels={channels}
            channelsLoading={slackReady && channels.length === 0 && !channelsError}
            onDone={refresh}
            onGoToInbox={() => setTab("inbox")}
          />
        </section>

        <section
          className="view"
          role="tabpanel"
          id="view-setup"
          aria-labelledby="tab-setup"
          hidden={tab !== "setup"}
        >
          <Setup setup={setup} onDone={refresh} onDoneJobs={() => setTab("jobs")} />
        </section>

        <footer className="colophon">
          <span className="sr-only">{inboxCount} drafts are waiting for approval.</span>
          <span aria-hidden="true">
            {ready ? "board live" : "connecting…"} · nothing is posted without your
            approval
          </span>
        </footer>
      </main>

      {/*
        Setup, always reachable from anywhere, without repeating itself in a banner the
        user reads several times a day. Bottom-left because that corner holds nothing on
        any of the three views, and the spine column ends well above it.
      */}
      <button
        className={`setup-float${setupNeedsAttention ? " setup-float--attention" : ""}`}
        onClick={goToSetup}
      >
        Setup
        {setupNeedsAttention && (
          <span className="sr-only"> — something here needs attention</span>
        )}
      </button>
    </div>
  );
}

createRoot(document.getElementById("root")).render(
  <StrictMode>
    <App />
  </StrictMode>,
);