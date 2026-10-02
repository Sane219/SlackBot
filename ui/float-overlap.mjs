// Does the floating Setup button ever sit on top of something the user needs to read?
//
// Fixed bottom-left is inherently an overlay, so the question is not "does it overlap"
// but "does it overlap at rest, when the user has stopped scrolling". That is the case
// that matters, and it is measured at the bottom of every scroll position on every tab.
//
//   node ui/float-overlap.mjs
import puppeteer from "puppeteer-core";

const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const URL = process.env.SB_URL || "http://127.0.0.1:7321/";
const OUT = "/tmp/sbtest";

const browser = await puppeteer.launch({
  executablePath: CHROME,
  headless: "new",
  args: ["--no-sandbox"],
});

const results = [];

for (const [w, h] of [
  [1440, 900],
  [1024, 768],
  [390, 844],
]) {
  const page = await browser.newPage();
  await page.setViewport({ width: w, height: h, deviceScaleFactor: 2 });
  await page.goto(URL, { waitUntil: "networkidle0" });
  await page.waitForSelector(".setup-float", { timeout: 10_000 });

  for (const tab of ["Inbox", "Jobs", "Setup"]) {
    await page.evaluate((t) => {
      const el = [...document.querySelectorAll('[role="tab"]')].find((b) =>
        b.textContent.trim().toLowerCase().startsWith(t.toLowerCase()),
      );
      el?.click();
    }, tab);
    await new Promise((r) => setTimeout(r, 350));

    // Two separate questions, because they have different answers:
    //
    //   atRest — the page is scrolled to the end and the user has stopped. Nothing may
    //            be hidden here; there is no further scroll to reveal it.
    //   midScroll — the user is partway down. A fixed control covers whatever passes
    //            beneath it, which is inherent to being fixed and is true of every
    //            floating button. Reported, not failed, so the cost stays visible.
    const measured = await page.evaluate(() => {
      const float = document.querySelector(".setup-float");
      const r = float.getBoundingClientRect();
      const scroll = document.scrollingElement;

      // `elementsFromPoint` returns outermost-first, so taking the first hit reports
      // `.spine` for anything anywhere in the spine — its textContent aggregates every
      // row. The question is what leaf is under the cursor, which is the *last* entry.
      const textUnder = () => {
        const stack = document
          .elementsFromPoint(r.left + r.width / 2, r.top + r.height / 2)
          .filter((el) => el !== float && !float.contains(el))
          .filter((el) => getComputedStyle(el).display !== "none" && el.getClientRects().length > 0);
        // A leaf: an element whose *own* text is non-empty. An ancestor's textContent is
        // the sum of its subtree, so `<html>` always "has text" and would report every
        // page as covered.
        return [...stack].reverse().find((el) =>
          [...el.childNodes].some(
            (n) => n.nodeType === Node.TEXT_NODE && n.textContent.trim().length > 0,
          ),
        );
      };

      const describe = (el, y) =>
        el
          ? {
              y,
              over: `${el.tagName}.${el.className || "(no class)"}`,
              sample: el.textContent.trim().slice(0, 50),
            }
          : null;

      const max = scroll.scrollHeight - window.innerHeight;
      scroll.scrollTop = max;
      const atRest = describe(textUnder(), max);

      const step = Math.max(120, Math.floor(window.innerHeight / 3));
      let midScroll = 0;
      let sample = null;
      for (let y = 0; y < max; y += step) {
        scroll.scrollTop = y;
        const el = textUnder();
        if (el) {
          midScroll += 1;
          sample ??= describe(el, y);
        }
      }

      scroll.scrollTop = 0;
      return { atRest, midScrollPositions: midScroll, sample, maxScroll: max };
    });

    results.push({ width: w, tab, ...measured });
    if (w === 390 && tab === "Inbox") {
      await page.evaluate(() => (document.scrollingElement.scrollTop = 0));
      await page.screenshot({ path: `${OUT}/float-${w}-${tab}.png` });
    }
  }
  await page.close();
}

console.log(JSON.stringify(results, null, 2));
const bad = results.filter((r) => r.atRest);
console.log(
  bad.length
    ? `HIDES TEXT AT REST:\n${bad
        .map((b) => `  ${b.width}px ${b.tab}: ${b.atRest.over} — "${b.atRest.sample}"`)
        .join("\n")}`
    : "at rest, on every tab at every width: nothing is hidden behind the button",
);
const transient = results.filter((r) => r.midScrollPositions > 0);
console.log(
  transient.length
    ? `passes over text mid-scroll (inherent to a fixed control): ${transient
        .map((t) => `${t.width}px ${t.tab} (${t.midScrollPositions} positions)`)
        .join(", ")}`
    : "no mid-scroll overlap either",
);
await browser.close();