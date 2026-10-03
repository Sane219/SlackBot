// Drive the interactive paths and screenshot each one. Not a substitute for a human
// looking, but it reaches states a first paint never shows.
//
//   node ui/drive.mjs
import puppeteer from "puppeteer-core";
import { writeFileSync } from "node:fs";

const CHROME = process.env.SB_CHROME || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const URL = process.env.SB_URL || "http://127.0.0.1:7321/";
const OUT = "/tmp/sbtest";

const browser = await puppeteer.launch({
  executablePath: CHROME,
  headless: "new",
  defaultViewport: { width: 1440, height: 1000, deviceScaleFactor: 2 },
  args: ["--no-sandbox", "--font-render-hinting=none"],
});
const page = await browser.newPage();

const problems = [];
page.on("console", (m) => {
  if (m.type() === "error" || m.type() === "warning") problems.push(`${m.type()}: ${m.text()}`);
});
page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
page.on("dialog", async (d) => {
  // Regenerate asks for confirmation. Accept it, and remember that it did.
  problems.push(`dialog: ${d.message().slice(0, 60)}…`);
  await d.accept();
});

const log = [];
const step = async (name, fn) => {
  try {
    log.push({ name, ...(await fn()) });
  } catch (err) {
    log.push({ name, error: err.message });
  }
  await new Promise((r) => setTimeout(r, 450));
  await page.screenshot({ path: `${OUT}/${name}.png` });
};

const clickText = (text, sel = "button") =>
  page.evaluate(
    (t, s) => {
      const el = [...document.querySelectorAll(s)].find((b) =>
        b.textContent.trim().toLowerCase().includes(t.toLowerCase()),
      );
      if (!el) throw new Error(`no ${s} matching "${t}"`);
      el.click();
      return el.textContent.trim();
    },
    text,
    sel,
  );

await page.goto(URL, { waitUntil: "networkidle0" });

// 1. The Evidence disclosure.
await step("evidence-open", async () => {
  await clickText("evidence");
  await new Promise((r) => setTimeout(r, 200));
  return {
    panelShown: await page.evaluate(() => Boolean(document.querySelector(".evidence"))),
    text: await page.evaluate(
      () => document.querySelector(".evidence")?.textContent.slice(0, 80) ?? null,
    ),
  };
});

// 2. Editing a draft in place.
await step("draft-editing", async () => {
  await clickText("edit");
  const ta = await page.$(".draft-card__edit");
  if (!ta) throw new Error("no textarea appeared");
  await page.evaluate(() => {
    const el = document.querySelector(".draft-card__edit");
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLTextAreaElement.prototype,
      "value",
    ).set;
    setter.call(el, el.value.replace("Auth rewrite", "Auth rewrite (edited)"));
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
  return {
    showsSaveEdit: await page.evaluate(() =>
      [...document.querySelectorAll("button")].some((b) =>
        b.textContent.includes("Save edit"),
      ),
    ),
    textKeystroke: await page.$eval(".draft-card__edit", (el) => el.value.slice(0, 30)),
  };
});

await step("draft-edit-saved", async () => {
  await clickText("save edit");
  await new Promise((r) => setTimeout(r, 600));
  return {
    bodyText: await page.$eval(".draft-card__body", (el) => el.textContent.slice(0, 30)),
    stillEditing: await page.evaluate(() => Boolean(document.querySelector(".draft-card__edit"))),
  };
});

// 3. Approve with no Slack configured. Must fail loudly and keep the Draft.
await step("approve-without-slack", async () => {
  await clickText("approve");
  await new Promise((r) => setTimeout(r, 700));
  return {
    note: await page.evaluate(() => document.querySelector(".banner")?.textContent ?? null),
    draftStillThere: await page.evaluate(() => document.querySelectorAll(".draft-card").length),
  };
});

// 4. The by-hand Job form.
await step("add-by-hand", async () => {
  await page.evaluate(() => {
    [...document.querySelectorAll('[role="tab"]')]
      .find((t) => t.textContent.trim() === "Jobs")
      .click();
  });
  await clickText("add one by hand");
  const fields = await page.evaluate(
    () => document.querySelectorAll(".manual .proposal__grid label").length,
  );
  const channel = await page.evaluate(() => {
    const s = document.querySelector(".manual select");
    return s ? { options: [...s.options].map((o) => o.text), disabled: s.disabled } : null;
  });
  return { fields, channel };
});

// 5. A time field narrowed to the right width.
await step("add-by-hand-filled", async () => {
  await page.evaluate(() => {
    const set = (el, v) => {
      const proto = el.tagName === "TEXTAREA" ? HTMLTextAreaElement : HTMLInputElement;
      Object.getOwnPropertyDescriptor(proto.prototype, "value").set.call(el, v);
      el.dispatchEvent(new Event("input", { bubbles: true }));
    };
    const labels = [...document.querySelectorAll(".manual .proposal__grid label")];
    const byName = (n) => labels.find((l) => l.querySelector("span").textContent.trim() === n);
    set(byName("Name").querySelector("input"), "Blocker write-up");
    set(byName("Time").querySelector("input"), "16:45");
    set(byName("Timezone").querySelector("input"), "Europe/London");
    set(byName("Instructions for writing this post").querySelector("textarea"), "One paragraph.");
  });
  return {
    timeFieldWidth: await page.$eval(".field--time", (el) =>
      Math.round(el.getBoundingClientRect().width),
    ),
    values: await page.evaluate(() =>
      [...document.querySelectorAll(".manual input, .manual textarea")].map((el) => el.value),
    ),
  };
});

// 6. Keyboard: every control reachable, every tab stop visible.
await step("focus-rings", async () => {
  return page.evaluate(() => {
    const focusable = [
      ...document.querySelectorAll(
        'button, a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      ),
    ].filter((el) => !el.disabled && el.offsetParent !== null);
    const invisible = focusable.filter((el) => {
      el.focus();
      const s = getComputedStyle(el);
      return s.outlineStyle === "none" && !s.boxShadow;
    });
    return {
      focusable: focusable.length,
      withoutRing: invisible.map((el) => el.textContent.trim().slice(0, 24) || el.tagName),
    };
  });
});

writeFileSync(`${OUT}/drive.json`, JSON.stringify({ log, problems }, null, 2));
console.log(JSON.stringify(log, null, 2));
console.log(problems.length ? `PROBLEMS:\n  ${problems.join("\n  ")}` : "console: clean");
await browser.close();