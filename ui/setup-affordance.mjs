// The Setup affordance: shown once in the banner on a fresh open, then carried by the
// floating button alone.
//
//   node ui/setup-affordance.mjs
import puppeteer from "puppeteer-core";

const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const URL = process.env.SB_URL || "http://127.0.0.1:7321/";

const browser = await puppeteer.launch({
  executablePath: CHROME,
  headless: "new",
  defaultViewport: { width: 1440, height: 900, deviceScaleFactor: 2 },
  args: ["--no-sandbox"],
});
const page = await browser.newPage();

const state = () =>
  page.evaluate(() => {
    const float = document.querySelector(".setup-float");
    const link = document.querySelector(".banner .linkish");
    const banner = document.querySelector(".banner--sev1");
    return {
      bannerPresent: Boolean(banner),
      bannerText: banner?.textContent.trim() ?? null,
      inlineLink: link ? link.textContent.trim() : null,
      floating: float
        ? {
            text: float.textContent.trim(),
            attention: float.classList.contains("setup-float--attention"),
            rect: (() => {
              const r = float.getBoundingClientRect();
              return {
                left: Math.round(r.left),
                bottom: Math.round(r.bottom),
                w: Math.round(r.width),
              };
            })(),
          }
        : null,
      // Does the floating button sit on top of anything readable?
      overlap: (() => {
        if (!float) return null;
        const r = float.getBoundingClientRect();
        const hit = document
          .elementsFromPoint(r.left + r.width / 2, r.top + r.height / 2)
          .filter((el) => el !== float && !float.contains(el));
        return hit.map((el) => el.className || el.tagName).slice(0, 3);
      })(),
    };
  });

const log = [];
await page.goto(URL, { waitUntil: "networkidle0" });
// React renders after the first poll resolves, which is after `networkidle0` reports
// quiet. Waiting for the element is the honest way to know it is there.
await page.waitForSelector(".setup-float", { timeout: 10_000 });
log.push({ when: "fresh load", ...(await state()) });

// Dismiss it by going to Setup — the button should do exactly that.
await page.click(".setup-float");
await new Promise((r) => setTimeout(r, 400));
log.push({ when: "after clicking the floating button", ...(await state()) });

// Back to the Inbox: the banner is still there, the inline link is not.
// The tab is a button whose text is "Inbox" plus, when Drafts are waiting, a count — so
// matching on exact text finds nothing and `undefined.click()` is the symptom.
await page.evaluate(() => {
  const tab = [...document.querySelectorAll('[role="tab"]')].find((t) =>
    t.textContent.trim().toLowerCase().startsWith("inbox"),
  );
  if (!tab) throw new Error("no Inbox tab");
  tab.click();
});
await new Promise((r) => setTimeout(r, 300));
log.push({ when: "back on the Inbox", ...(await state()) });

// A reload is a fresh open, so the link comes back.
await page.reload({ waitUntil: "networkidle0" });
await new Promise((r) => setTimeout(r, 400));
log.push({ when: "after a reload", ...(await state()) });

console.log(JSON.stringify(log, null, 2));
await browser.close();