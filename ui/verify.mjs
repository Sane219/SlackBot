// Run the browser checks against a running server, from the UI directory.
//
// These are not a substitute for looking at the page — one bug here was invisible to a
// DOM probe and obvious in a screenshot — but they make the things a probe cannot see
// cheap to re-check after every change.
//
//   cargo run
//   node ui/verify.mjs   # every check
//
// `shot.mjs` and `drive.mjs` stay separate: they write screenshots for a human.

import { existsSync } from "node:fs";
import puppeteer from "puppeteer-core";

// Chrome, wherever this is running. Hardcoding the macOS path meant the script could
// only ever work on the machine that wrote it — and CI runs on Linux.
const CHROME = process.env.SB_CHROME || resolveChrome();

/** First known Chrome path that exists. */
function resolveChrome() {
  const known = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
  ];
  const found = known.find((p) => existsSync(p));
  if (!found) {
    throw new Error(
      `no Chrome found. Set SB_CHROME to a Chrome or Chromium binary. Tried:\n  ${known.join("\n  ")}`,
    );
  }
  return found;
}
const URL = process.env.SB_URL || "http://127.0.0.1:7321/";
const WIDTHS = [
  [1440, 900],
  [1024, 768],
  [390, 844],
];

const results = [];
const check = (group, name, pass, detail) =>
  results.push({ group, name, pass, detail: detail ?? null });

/**
 * How many checks ran, so "all passed" cannot mean "none ran".
 *
 * The spine check once read from a variable that had been deleted in an earlier edit. It
 * produced `undefined`, compared two `undefined`s, and passed — so a check silently
 * stopped checking while still reporting green. A check that cannot fail is a lie told at
 * the most convenient moment. This is the cheap guard: a group that contributes nothing
 * fails the run.
 */
const EXPECTED_GROUPS = ["layout", "spine", "a11y", "setup", "console", "tabs"];

/**
 * How many Fire rows the spine should be showing.
 *
 * Read from the API rather than hardcoded, because the daemon is live: it fires Jobs while
 * the checks run, so a fixed count is wrong within a minute. Asserted rather than
 * tolerated, because a check that passes on an empty selection passes for the wrong
 * reason — that is the exact failure this file exists to catch.
 */
async function expectedFireRows() {
  const res = await fetch(`${URL}api/inbox`);
  const body = await res.json();
  return body.fires.length;
}

const browser = await puppeteer.launch({
  executablePath: CHROME,
  headless: "new",
  args: ["--no-sandbox", "--font-render-hinting=none"],
});

// Read once, before the widths loop: the daemon is live and fires Jobs while these run.
const expected = await expectedFireRows();

for (const [width, height] of WIDTHS) {
  const page = await browser.newPage();
  const problems = [];
  page.on("console", (m) => {
    if (m.type() === "error" || m.type() === "warning") problems.push(`${m.type()}: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  page.on("requestfailed", (r) => problems.push(`requestfailed: ${r.url()}`));
  page.on("response", (r) => {
    // 412 from /api/channels is expected when Slack is not configured.
    if (r.status() >= 400 && !r.url().includes("/api/channels")) {
      problems.push(`http ${r.status()}: ${r.url()}`);
    }
  });

  await page.setViewport({ width, height, deviceScaleFactor: 1 });
  await page.goto(URL, { waitUntil: "networkidle0" });
  await page.waitForSelector(".setup-float", { timeout: 10_000 });
  await new Promise((r) => setTimeout(r, 400));

  const tag = `${width}px`;

  // No horizontal scroll. A board is read, not browsed sideways.
  const overflow = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    innerWidth: window.innerWidth,
  }));
  check(
    "layout",
    `${tag} no horizontal overflow`,
    overflow.scrollWidth <= overflow.innerWidth + 1,
    `${overflow.scrollWidth} > ${overflow.innerWidth}`,
  );

  // Exactly one view is shown. `display: flex` beating the `hidden` attribute rendered
  // every tab at once once, and only a computed-style check catches it.
  const panels = await page.evaluate(() =>
    [...document.querySelectorAll('[role="tabpanel"]')].map((p) => ({
      id: p.id,
      hiddenAttr: p.hidden,
      display: getComputedStyle(p).display,
    })),
  );
  const shown = panels.filter((p) => p.display !== "none");
  check(
    "layout",
    `${tag} exactly one view visible`,
    shown.length === 1,
    shown.map((p) => `${p.id}=${p.display}`).join(" "),
  );
  check(
    "layout",
    `${tag} the hidden attribute and the computed style agree`,
    panels.every((p) => (p.hiddenAttr ? p.display === "none" : p.display !== "none")),
    JSON.stringify(panels),
  );

  // Every spine row that stands for a recorded Fire shows a time. A prop mismatch
  // (`clock: at` against callers passing `at`) silently rendered the whole column blank.
  //
  // Two row kinds are excluded, each for a reason. The placeholders ("connecting…", "no
  // fires yet") have no Fire behind them, and counting them would fail on a fresh install
  // -- the first state a contributor sees. `NEXT` is what is coming rather than what
  // happened; it renders a time and is checked on its own below, but it is not one of the
  // Fires the API reports.
  const clocked = await page.evaluate(() =>
    [...document.querySelectorAll(".fire")]
      .filter((r) => !r.querySelector(".fire__code")?.textContent.includes("SYS"))
      .filter((r) => !r.querySelector(".fire__code")?.textContent.includes("NEXT"))
      .map((r) => Boolean(r.querySelector(".fire__clock"))),
  );
  check(
    "spine",
    `${tag} every Fire row shows a time`,
    // No `|| clocked.length === 0`. An empty selection used to satisfy this by vacuous
    // truth, so a broken selector made the check pass *by checking nothing* -- which is
    // indistinguishable from a working check, and is the failure this file exists to
    // catch. The count is asserted against the API instead.
    clocked.every(Boolean) && clocked.length === expected,
    `${clocked.filter((c) => !c).length} without a clock, of ${clocked.length} (API reports ${expected})`,
  );

  // And the row for what comes next shows one too, since the time is its entire value.
  const nextRow = await page.evaluate(
    () =>
      [...document.querySelectorAll(".fire")]
        .find((r) => r.querySelector(".fire__code")?.textContent.includes("NEXT"))
        ?.querySelector(".fire__clock")?.textContent ?? null,
  );
  check(
    "spine",
    `${tag} the NEXT row shows when it fires`,
    nextRow === null || /\d{2}:\d{2}/.test(nextRow),
    nextRow,
  );

  // Every tab stop has a visible focus ring. WCAG AA, and only a browser can tell --
  // a computed-style probe is what the last release relied on and it was wrong.
  const rings = await page.evaluate(() => {
    const focusable = [
      ...document.querySelectorAll(
        'button, a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      ),
    ].filter((el) => !el.disabled && el.offsetParent !== null);
    const bare = focusable.filter((el) => {
      el.focus();
      const s = getComputedStyle(el);
      return s.outlineStyle === "none" && s.boxShadow === "none";
    });
    return { total: focusable.length, bare: bare.map((el) => el.textContent.trim().slice(0, 24) || el.tagName) };
  });
  check(
    "a11y",
    `${tag} every tab stop has a focus ring`,
    rings.total > 0 && rings.bare.length === 0,
    rings.total === 0 ? "no tab stops found -- the selector is stale" : rings.bare.join(", "),
  );

  // The Setup control must stay reachable from anywhere, and must not cover text the user
  // came to read once they have stopped scrolling.
  const float = await page.evaluate(() => {
    const el = document.querySelector(".setup-float");
    const r = el.getBoundingClientRect();
    const scroll = document.scrollingElement;
    scroll.scrollTop = scroll.scrollHeight;
    const stack = document
      .elementsFromPoint(r.left + r.width / 2, r.top + r.height / 2)
      .filter((n) => n !== el && !el.contains(n));
    const leaf = [...stack]
      .reverse()
      .find((n) =>
        [...n.childNodes].some(
          (c) => c.nodeType === Node.TEXT_NODE && c.textContent.trim().length > 0,
        ),
      );
    scroll.scrollTop = 0;
    return {
      isButton: el.tagName === "BUTTON",
      tabbable: el.tabIndex >= 0,
      hidesText: leaf ? leaf.textContent.trim().slice(0, 40) : null,
    };
  });
  check(
    "setup",
    `${tag} the Setup control is a real button`,
    float.isButton && float.tabbable,
    `button=${float.isButton} tabbable=${float.tabbable}`,
  );
  check("setup", `${tag} nothing hidden behind Setup at rest`, !float.hidesText, float.hidesText);

  // A gap is a picture *and* a sentence: the hatch is decorative and aria-hidden, and
  // an sr-only span beside it states that nothing was collected. Checked together, so a
  // hatch can never ship without its sentence.
  const gap = await page.evaluate(() => {
    const hatches = document.querySelectorAll(".fire .hatch").length;
    const spoken = document.querySelectorAll(".fire .sr-only").length;
    return { hatches, spoken };
  });
  check(
    "spine",
    `${tag} each hatch has a spoken sentence`,
    gap.hatches <= gap.spoken,
    `${gap.hatches} hatches, ${gap.spoken} spoken sentences`,
  );

  check("console", `${tag} no console errors or failed requests`, problems.length === 0, problems.join(" | "));
  await page.close();
}

// Every tab, once, at the default width — the states that only exist after interaction.
{
  const page = await browser.newPage();
  const problems = [];
  page.on("pageerror", (e) => problems.push(e.message));
  await page.setViewport({ width: 1440, height: 900, deviceScaleFactor: 1 });
  await page.goto(URL, { waitUntil: "networkidle0" });
  await page.waitForSelector(".setup-float", { timeout: 10_000 });

  for (const tab of ["Jobs", "Setup", "Inbox"]) {
    await page.evaluate((t) => {
      [...document.querySelectorAll('[role="tab"]')]
        .find((b) => b.textContent.trim().toLowerCase().startsWith(t.toLowerCase()))
        ?.click();
    }, tab);
    await new Promise((r) => setTimeout(r, 300));
    const shown = await page.evaluate(() =>
      [...document.querySelectorAll('[role="tabpanel"]')]
        .filter((p) => getComputedStyle(p).display !== "none")
        .map((p) => p.id),
    );
    check("tabs", `clicking ${tab} shows exactly one view`, shown.length === 1, shown.join());
  }
  check("tabs", "no page errors while switching tabs", problems.length === 0, problems.join(" | "));
  await page.close();
}

await browser.close();

// Every group must contribute at least one check. A group that produced none means its
// checks stopped running — a deleted variable, a renamed selector, a filter that no longer
// matches — and that is indistinguishable from passing unless it is made to fail.
const ran = new Set(results.map((r) => r.group));
const missing = EXPECTED_GROUPS.filter((g) => !ran.has(g));
if (missing.length) {
  console.log(
    `FAIL  coverage  these groups ran no checks at all, so something stopped executing: ${missing.join(", ")}`,
  );
}

const failed = results.filter((r) => !r.pass);
for (const r of results) {
  if (!r.pass) console.log(`FAIL  ${r.group}  ${r.name}  ${r.detail ?? ""}`);
}
console.log(
  `\n${results.length - failed.length}/${results.length} checks passed` +
    (failed.length ? ` — ${failed.length} failing` : "") +
    (missing.length ? ` — ${missing.length} group(s) silently skipped` : ""),
);
process.exit(failed.length || missing.length ? 1 : 0);