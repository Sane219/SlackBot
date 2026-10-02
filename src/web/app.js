// The board, wired to the local API.
//
// Nothing here posts to Slack. The only write is the Approve click, which mirrors the
// server's own guarantee: there is no code path from this page that skips a human.

const state = { view: "inbox", drafts: [], fires: [], jobs: [], present: null };

// ── helpers ─────────────────────────────────────────────────────────────────

const $ = (sel) => document.querySelector(sel);

function el(tag, props = {}, ...children) {
  const node = Object.assign(document.createElement(tag), props);
  for (const child of children.flat()) {
    if (child == null) continue;
    node.append(child);
  }
  return node;
}

async function api(path, options = {}) {
  const res = await fetch(path, {
    headers: { "Content-Type": "application/json" },
    ...options,
    body: options.body ? JSON.stringify(options.body) : undefined,
  });
  const text = await res.text();
  // An HTML error page must not be reported as a parse error: the status says more.
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    throw new Error("the server returned a response this page could not read");
  }
  if (!res.ok) {
    throw new Error((data && data[1]) || `HTTP ${res.status}`);
  }
  return data;
}

// ── the spine ───────────────────────────────────────────────────────────────

// One row per Fire, in schedule order, with the classification code. A gap is drawn as a
// gap: hatched, with its span printed.
function renderSpine() {
  const host = $("#spine-rows");
  host.replaceChildren();

  if (!state.fires.length) {
    host.append(
      el("div", { className: "fire" },
        el("span", { className: "fire__rule", ariaHidden: "true" }),
        el("div", { className: "fire__name" }, "no fires yet"),
        el("div", { className: "fire__code" }, "SYS"),
      ),
    );
    return;
  }

  for (const fire of state.fires) {
    const missed = fire.outcome === "missed";
    const failed = fire.outcome === "failed";
    const noSignal = fire.no_signal;

    const classes = ["fire"];
    if (failed) classes.push("fire--failed");
    else if (missed) classes.push("fire--nosignal");
    else if (!noSignal) classes.push("fire--traced");

    const row = el("div", { className: classes.join(" ") });
    row.append(el("span", { className: "fire__rule", ariaHidden: "true" }));
    row.append(el("div", { className: "fire__clock" }, clockOf(fire.fired_at)));
    row.append(el("div", { className: "fire__name" }, fire.job_name));

    const code = failed && fire.error
      ? `SEV1 · ${shortError(fire.error)}`
      : fire.outcome === "missed"
        ? "MIS"
        : codeOf(fire.outcome);
    row.append(el("div", { className: "fire__code" }, code));

    if (noSignal) {
      const hatch = el("div", { className: "hatch" });
      hatch.setAttribute("aria-hidden", "true");
      row.append(hatch);
      // Spoken, because a hatch is a picture and a gap is a fact.
      row.append(el("span", { className: "sr-only" },
        missed
          ? "The app was not running at this time, so nothing was collected."
          : "No signal. Nothing was collected in this window."));
    }

    host.append(row);
  }
}

// "Asia/Kolkata" -> "Kolkata"; "UTC" and "GMT" have no slash and must not render as
// "undefined".
const tzLabel = (tz) => (tz.includes("/") ? tz.split("/")[1] : tz);

const codeOf = (outcome) =>
  ({ drafted: "ACT", partial: "PR", failed: "SEV1", missed: "MIS", skipped: "SKP" }[outcome] || "SYS");

const shortError = (err) => String(err).split("\n")[0].slice(0, 48);

function clockOf(iso) {
  const d = new Date(iso);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

// ── the Inbox ───────────────────────────────────────────────────────────────

function renderInbox() {
  const host = $("#draft-host");
  host.replaceChildren();

  if (!state.drafts.length) {
    host.append(el("div", { className: "empty" },
      "Nothing waiting. A Draft appears here when a Job fires."));
    return;
  }

  for (const draft of state.drafts) {
    // Leave a card the user is editing alone until they are done with it.
    if (beingEdited.has(draft.id)) continue;

    const card = el("div", { className: `draft-card${draft.no_signal ? " draft-card--nosignal" : ""}` });

    card.append(el("div", { className: "draft-card__head" },
      el("div", { className: "draft-card__name" }, draft.job_name),
      el("div", { className: "window" },
        `${stamp(draft.window_from)} → ${stamp(draft.window_to)}`)));

    // Counts in code, never in prose.
    const counts = el("div", { className: "counts" });
    const c = draft.counts || {};
    for (const [code, value] of [["MSG", c.slack_messages], ["PR", c.github_comments], ["GIT", c.github_comments]]) {
      if (!value) continue;
      counts.append(el("div", { className: "count" },
        el("span", { className: "count__code" }, code),
        el("span", { className: "count__n" }, String(value))));
    }
    // Truncation states itself rather than hiding in the counts.
    if (c.slack_messages < c.slack_fetched || c.github_comments < c.github_fetched) {
      counts.append(el("div", { className: "count grain" },
        `trimmed: ${c.slack_messages}/${c.slack_fetched} msg, ${c.github_comments}/${c.github_fetched} comments`));
    }
    card.append(counts);

    const body = el("div", { className: "draft-card__body" }, draft.text);
    card.append(body);

    const actions = el("div", { className: "actions" });
    const rerender = () => refresh();

    if (draft.no_signal) {
      // A gap has nothing to approve. Approving it would put NO SIGNAL in a channel.
      actions.append(el("span", { className: "grain" },
        "Nothing was collected, so there is nothing to post."));
      actions.append(el("div", { className: "actions__gap", ariaHidden: "true" }));
      actions.append(button("Discard", "btn btn--secondary", () => discard(draft.id, rerender)));
    } else {
      const textarea = el("textarea", { rows: 6, value: draft.text });
      textarea.value = draft.text;
      card.append(textarea);

      textarea.addEventListener("focus", () => beingEdited.add(draft.id));
      textarea.addEventListener("blur", () => beingEdited.delete(draft.id));

      const approve = button("Approve & send", "btn btn--approve", async (ev) => {
        ev.target.disabled = true;
        try {
          const result = await api(`/api/drafts/${draft.id}/approve`, { method: "POST" });
          if (result.note) alert(result.note);
        } catch (err) {
          alert(`Could not send: ${err.message}`);
        }
        rerender();
      });
      actions.append(approve);
      actions.append(button("Save edit", "btn btn--secondary", async (ev) => {
        ev.target.disabled = true;
        try {
          await api(`/api/drafts/${draft.id}`, { method: "PATCH", body: { text: textarea.value } });
          beingEdited.delete(draft.id);
        } catch (err) {
          alert(`Could not save: ${err.message}`);
        }
        rerender();
      }));
      actions.append(button("Regenerate", "btn btn--secondary", async (ev) => {
        // Ask first, and only then disable: disabling before the confirm left the button
        // dead for the life of the card if the user cancelled.
        // Re-fetching re-reads the original window, so the Draft's meaning cannot drift.
        if (!confirm("Re-fetch the original window and draft again? The result may differ if messages have changed since.")) return;
        ev.target.disabled = true;
        try {
          await api(`/api/drafts/${draft.id}/regenerate`, { method: "POST" });
        } catch (err) {
          alert(`Could not regenerate: ${err.message}`);
        }
        rerender();
      }));
      actions.append(el("div", { className: "actions__gap", ariaHidden: "true" }));
      actions.append(button("Discard", "btn btn--secondary", () => discard(draft.id, rerender)));
    }

    card.append(actions);
    host.append(card);
  }
}

const button = (label, className, onClick) => {
  const b = el("button", { className, type: "button" }, label);
  b.addEventListener("click", onClick);
  return b;
};

async function discard(id, rerender) {
  await api(`/api/drafts/${id}`, { method: "DELETE" });
  rerender();
}

const stamp = (iso) => {
  const d = new Date(iso);
  return `${d.getDate()} ${d.toLocaleString("en", { month: "short" })} ${clockOf(iso)}`;
};

// ── Jobs ────────────────────────────────────────────────────────────────────

function renderJobs() {
  const host = $("#jobs-host");
  host.replaceChildren();

  if (!state.jobs.length) {
    host.append(el("div", { className: "empty" },
      "No Jobs yet. Describe your routine below and check the proposal before saving it."));
    return;
  }

  for (const job of state.jobs) {
    const when = `${job.schedule.at} ${tzLabel(job.schedule.tz)}`;
    const window_ = job.context_window.kind === "lookback"
      ? `last ${job.context_window.hours}h`
      : `since ${job.context_window.at}${job.context_window.previous_day ? " prev day" : ""}`;

    const row = el("div", { className: "step" });
    row.append(el("div", { className: "step__code" }, job.enabled ? "ON" : "OFF"));
    const body = el("div", { className: "step__body" });
    body.append(el("div", { className: "step__label" }, job.name));
    body.append(el("div", { className: "step__note" },
      `${when} · ${window_} · #${job.channel.name}`));
    row.append(body);

    const actions = el("div", { className: "step__row" });
    actions.append(button("Fire now", "btn btn--secondary", async (ev) => {
      ev.target.disabled = true;
      try {
        await api(`/api/jobs/${job.id}/fire`, { method: "POST" });
      } catch (err) {
        alert(`Could not fire: ${err.message}`);
      }
      await refresh();
      show("inbox");
    }));
    actions.append(button(job.enabled ? "Disable" : "Enable", "btn btn--secondary", async () => {
      await api(`/api/jobs/${job.id}`, { method: "PATCH", body: { enabled: !job.enabled } });
      await refresh();
    }));
    row.append(actions);
    host.append(row);
  }
}

// ── Setup ───────────────────────────────────────────────────────────────────

const CREDENTIALS = [
  { kind: "llm_api_key", code: "LLM", label: "Model API key", note: "Stored in the OS keychain. Never written to a file." },
  { kind: "slack_token", code: "SLK", label: "Slack token", note: "The xoxc… token. Stored in the keychain." },
  { kind: "slack_cookie", code: "SLK", label: "Slack d cookie", note: "Paste the cookie value exactly as the browser sends it, percent-escapes intact. Decoding it breaks auth." },
  { kind: "github_token", code: "GH", label: "GitHub token", note: "A PAT with read access. Stored in the keychain." },
];

function renderSetup() {
  const host = $("#setup-rows");
  host.replaceChildren();
  const present = state.present || {};

  const flags = {
    llm_api_key: present.llm,
    slack_token: present.slack_token,
    slack_cookie: present.slack_cookie,
    github_token: present.github,
  };

  for (const cred of CREDENTIALS) {
    const row = el("div", { className: "step" });
    row.append(el("div", { className: "step__code" }, cred.code));

    const body = el("div", { className: "step__body" });
    const input = el("input", { type: "password", placeholder: flags[cred.kind] ? "•••••••• stored" : "paste to store" });
    input.setAttribute("aria-label", cred.label);
    body.append(el("label", { className: "step__label" }, cred.label));
    body.append(input);
    body.append(el("div", { className: "step__note" }, cred.note));
    row.append(body);

    const actions = el("div", { className: "step__row" });
    const mark = el("div", { className: `step__mark${flags[cred.kind] ? "" : " step__mark--pending"}` },
      flags[cred.kind] ? "OK" : "PENDING");
    actions.append(mark);

    actions.append(button("Save", "btn btn--secondary", async () => {
      if (!input.value.trim()) return;
      try {
        await api("/api/setup/credential", { method: "POST", body: { kind: cred.kind, value: input.value } });
        input.value = "";
        await refresh();
      } catch (err) {
        alert(`Could not save: ${err.message}`);
      }
    }));

    actions.append(button("Verify", "btn btn--secondary", async (ev) => {
      const original = ev.target.textContent;
      ev.target.disabled = true;
      ev.target.textContent = "…";
      try {
        const result = await api("/api/setup/verify", { method: "POST", body: { kind: cred.kind } });
        mark.textContent = result.ok ? `OK${result.identity ? " · " + result.identity : ""}` : "FAILED";
        mark.className = result.ok ? "step__mark" : "step__mark";
        if (!result.ok) alert(result.reason);
        await refresh();
      } catch (err) {
        mark.textContent = "FAILED";
        alert(err.message);
      }
      ev.target.disabled = false;
      ev.target.textContent = original;
    }));

    row.append(actions);
    host.append(row);
  }

  // The endpoint and model are not secrets, so they are shown openly.
  const row = el("div", { className: "step" });
  row.append(el("div", { className: "step__code" }, "LLM"));
  const body = el("div", { className: "step__body" });
  const url = el("input", { type: "text", placeholder: "http://127.0.0.1:8000/v1" });
  url.setAttribute("aria-label", "Model endpoint");
  const model = el("input", { type: "text", placeholder: "model name" });
  model.setAttribute("aria-label", "Model name");
  body.append(el("label", { className: "step__label" }, "Endpoint and model"));
  body.append(url);
  body.append(model);
  body.append(el("div", { className: "step__note" },
    "Checked at save, so a bad endpoint fails here rather than at 14:30."));
  row.append(body);
  const actions = el("div", { className: "step__row" });
  actions.append(button("Save", "btn btn--secondary", async () => {
    try {
      await api("/api/setup/llm", { method: "POST", body: { base_url: url.value, model: model.value } });
      await refresh();
    } catch (err) {
      alert(`Could not save: ${err.message}`);
    }
  }));
  row.append(actions);
  host.append(row);
}

// ── navigation ──────────────────────────────────────────────────────────────

function show(view) {
  state.view = view;
  for (const name of ["inbox", "jobs", "setup"]) {
    $(`#view-${name}`).hidden = name !== view;
  }
  for (const tab of document.querySelectorAll(".tab")) {
    tab.setAttribute("aria-selected", String(tab.dataset.view === view));
  }
  const labels = { inbox: "Fire", jobs: "Job", setup: "Channel" };
  $("#spine-label").textContent = labels[view];
  $("#pane-title").textContent = view[0].toUpperCase() + view.slice(1);
  renderWindowLabel();
}

/** The window describes a Draft, so it only means anything on the Inbox. */
function renderWindowLabel() {
  const label = $("#window-label");
  const newest = state.drafts[0];
  const show = state.view === "inbox" && newest;
  label.hidden = !show;
  label.textContent = show ? `${stamp(newest.window_from)} → ${stamp(newest.window_to)}` : "";
}

for (const tab of document.querySelectorAll(".tab")) {
  tab.addEventListener("click", () => show(tab.dataset.view));
}

$("#plan-btn").addEventListener("click", async () => {
  const out = $("#plan-result");
  out.textContent = "asking…";
  try {
    const result = await api("/api/plan", { method: "POST", body: { description: $("#routine").value } });
    // The Plan Role proposes. Nothing is scheduled until each is confirmed below.
    let created = 0;
    const failed = [];
    for (const job of result.jobs) {
      const context = job.context === "since" ? "since" : "lookback";
      try {
        // Send the channel as written. The server resolves the name to Slack's C… id;
        // passing the name through as an id makes every read fail with channel_not_found
        // and every post 502.
        await api("/api/jobs", {
          method: "POST",
          body: {
            name: job.name, at: job.at, tz: job.tz,
            channel_id: job.channel,
            channel_name: job.channel.replace(/^#/, ""),
            context, since_at: job.since_at, previous_day: job.previous_day,
            lookback_hours: 8, prompt: job.prompt,
          },
        });
        created += 1;
      } catch (err) {
        failed.push(`${job.name} (${job.channel}): ${err.message}`);
      }
    }
    out.textContent = failed.length
      ? `${created} saved, ${failed.length} not: ${failed.join("; ")}`
      : `${created} job(s) saved. Check them below and edit what does not match.`;
    await refresh();
  } catch (err) {
    // An unusable proposal is shown raw rather than repaired into something the user
    // did not choose.
    out.textContent = err.message;
  }
});

// ── poll ────────────────────────────────────────────────────────────────────

// Cards the user is currently editing, keyed by draft id. A background poll must not
// rebuild a card someone is typing in: it wiped the text every 20 seconds unless the
// user clicked Save first, and cleared the disabled state of an in-flight Approve.
const beingEdited = new Set();

async function refresh() {
  try {
    const [inbox, jobs, setup] = await Promise.all([
      api("/api/inbox").catch(() => ({ drafts: [], fires: [] })),
      api("/api/jobs").catch(() => ({ jobs: [] })),
      api("/api/setup").catch(() => null),
    ]);
    state.drafts = inbox.drafts || [];
    state.fires = inbox.fires || [];
    state.jobs = jobs.jobs || [];
    state.present = setup ? setup.present : null;
  } catch {
    // A failed poll is not worth a modal. The spine keeps its last state.
    return;
  }

  renderSpine();
  renderInbox();
  renderJobs();
  renderSetup();

  renderWindowLabel();
  $("#spine-label").textContent = { inbox: "Fire", jobs: "Job", setup: "Channel" }[state.view];
}

refresh();
setInterval(refresh, 20000);
