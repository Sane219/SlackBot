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

const browser = await puppeteer.launch({
  executablePath: CHROME,
  headless: "new",
  args: ["--no-sandbox", "--font-render-hinting=none"],
});

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

  // Every spine row with a time shows one. A prop mismatch (`clock: at` against callers
  // passing `at`) silently rendered the whole column blank.
  const spine = await page.evaluate(() => {
    const rows = [...document.querySelectorAll(".fire")];
    return {
      rows: rows.length,
      withClock: rows.filter((r) => r.querySelector(".fire__clock")).length,
      hatches: document.querySelectorAll(".hatch").length,
      srOnly: document.querySelectorAll(".fire .sr-only").length,
    };
  });
  const clocked = await page.evaluate(
    () =>
      [...document.querySelectorAll(".fire")]
        .filter((r) => r.querySelector(".fire__code"))
        .map((r) => Boolean(r.querySelector(".fire__clock"))),
  );
  check(
    "spine",
    `${tag} every coded row shows a time`,
    clocked.length === 0 || clocked.every(Boolean),
    clocked.filter((c) => !c).length + " rows without a clock of " + clocked.length,
  );
  check(
    "spine",
    `${tag} each hatch has a spoken sentence`,
    spine.hatches <= spine.srOnly,
    `${spine.hatches} hatches, ${spine.srOnly} sr-only sentences`,
  );

  // Every tab stop has a visible focus ring. WCAG AA, and it was checked in a browser.
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
    rings.bare.length === 0,
    rings.bare.join(", "),
  );

  // The Setup button must not be the only way to reach Setup, and must not cover text
  // at rest.
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
      label: el.textContent.trim(),
      isButton: el.tagName === "BUTTON",
      tabbable: el.tabIndex >= 0,
      hidesText: leaf ? leaf.textContent.trim().slice(0, 40) : null,
    };
  });
  check("setup", `${tag} the Setup control is a real button`, float.isButton && float.tabbable);
  check(`${tag} nothing hidden at rest`, `Setup button hides "${float.hidesText}"`, !float.hidesText, float.hidesText);

  // Times must be rendered in the Job's own timezone. A Job in Los Angeles read 06:59
  // while its machine was in India — correct only because the zone is threaded through.
  const zones = await page.evaluate(() => {
    const out = [];
    for (const el of document.querySelectorAll(".job .step__note")) out.push(el.textContent);
    return out;
  });
  if (zones.length) {
    check(
      "timezone",
      `${tag} job rows name a zone`,
      zones.every((z) => /\d{2}:\d{2}\s+\S/.test(z)),
      zones[0],
    );
  }

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

const failed = results.filter((r) => !r.pass);
for (const r of results) {
  if (!r.pass) console.log(`FAIL  ${r.group}  ${r.name}  ${r.detail ?? ""}`);
}
console.log(
  `\n${results.length - failed.length}/${results.length} checks passed` +
    (failed.length ? ` — ${failed.length} failing` : ""),
);
process.exit(failed.length ? 1 : 0);