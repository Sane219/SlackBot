// Render the board in a real browser and screenshot it.
//
// Screenshots, not DOM assertions: a probe once reported `hidden` was set correctly
// while the element was plainly visible on screen, because a class beat the attribute.
// Only pixels settle it.
//
//   node ui/shot.mjs <label> [--tab=jobs|setup|inbox] [--w=1440] [--h=900] [--script=file.js]

import puppeteer from "puppeteer-core";
import { readFileSync, writeFileSync } from "node:fs";

const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const URL = process.env.SB_URL || "http://127.0.0.1:7321/";
const OUT = "/tmp/sbtest";

const args = Object.fromEntries(
  process.argv.slice(3).map((a) => {
    const [k, v] = a.replace(/^--/, "").split("=");
    return [k, v ?? true];
  }),
);

const label = process.argv[2] || "shot";
const width = Number(args.w || 1440);
const height = Number(args.h || 900);

const browser = await puppeteer.launch({
  executablePath: CHROME,
  headless: "new",
  defaultViewport: { width, height, deviceScaleFactor: 2 },
  args: ["--no-sandbox", "--font-render-hinting=none"],
});

const page = await browser.newPage();
const problems = [];
page.on("console", (m) => {
  if (m.type() === "error" || m.type() === "warning") {
    problems.push(`${m.type()}: ${m.text()}`);
  }
});
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
page.on("requestfailed", (r) => problems.push(`requestfailed: ${r.url()}`));
page.on("response", (r) => {
  if (r.status() >= 400) problems.push(`http ${r.status()}: ${r.url()}`);
});

await page.goto(URL, { waitUntil: "networkidle0" });

if (args.script) {
  const source = readFileSync(args.script, "utf8");
  const result = await page.evaluate(source);
  if (result !== undefined) writeFileSync(`${OUT}/${label}.json`, JSON.stringify(result, null, 2));
}

if (args.tab) {
  await page.evaluate((t) => {
    const tab = [...document.querySelectorAll('[role="tab"]')].find(
      (b) => b.textContent.trim().toLowerCase() === t,
    );
    tab?.click();
  }, args.tab);
}

await new Promise((r) => setTimeout(r, 700));
await page.screenshot({ path: `${OUT}/${label}.png`, fullPage: Boolean(args.full) });

// Report what is actually visible, not what the DOM claims.
const seen = await page.evaluate(() => {
  const box = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return `${sel}: absent`;
    const r = el.getBoundingClientRect();
    const s = getComputedStyle(el);
    return `${sel}: ${Math.round(r.width)}x${Math.round(r.height)} display=${s.display} visible=${
      s.display !== "none" && s.visibility !== "hidden" && r.height > 0
    }`;
  };
  const ink = getComputedStyle(document.body).backgroundColor;
  return {
    bodyBackground: ink,
    rootChildren: document.getElementById("root")?.children.length ?? -1,
    tabs: [...document.querySelectorAll('[role="tabpanel"]')]
      .map((p) => `${p.id}=${p.hidden ? "hidden" : "SHOWN"}`)
      .join(" "),
    boxes: [
      box(".board"),
      box(".spine"),
      box(".pane"),
      box(".tabs"),
      box(".view:not([hidden])"),
    ],
    overflowX: document.documentElement.scrollWidth > window.innerWidth,
    scrollWidth: document.documentElement.scrollWidth,
    innerWidth: window.innerWidth,
  };
});

console.log(JSON.stringify(seen, null, 2));
console.log(problems.length ? `PROBLEMS:\n  ${problems.join("\n  ")}` : "console: clean");

await browser.close();