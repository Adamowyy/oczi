// The health report window: everything one scan found, and the one repair it can offer.
//
// A window of its own rather than a bigger card, because the list is what a person works through —
// the island keeps the score and the three loudest findings, and sends them here for the rest. The
// scan is still the same one-off pass: pressing a button here runs it, and nothing survives the
// call except the report on disk.

import "../pc/pc.css";
import { Bridge, onEvent } from "../core/bridge";
import { setLang, t, type TextKey } from "../core/i18n";
import { h, clear } from "../views/dom";
import type { HealthFinding, HealthReport } from "../core/state";

const root = document.getElementById("pc-root")!;

let report: HealthReport | null = null;
let busy = false;

/** A finding as one line. The wording is a template per kind, so the report reads in whichever
 *  language the app is in — `subject` is the program or service, `value` its number. */
function findingText(f: HealthFinding): string {
  const key = `health.f.${f.kind}` as TextKey;
  return f.subject ? t(key, f.subject, f.value) : t(key, f.value);
}

function scoreColor(score: number): string {
  return score >= 90 ? "#22c55e" : score >= 70 ? "#f5a524" : "#f4505e";
}

function checkName(id: string): string {
  return t(`health.check.${id}` as TextKey);
}

function checkWord(state: string): string {
  return t(`health.check.${state}` as TextKey);
}

/** One finding, with the repair behind a confirmation the user has to press: nothing is closed
 *  because a scan said so, only because a person read the line and agreed. */
function findingRow(f: HealthFinding): HTMLElement {
  const result = h("div", { class: "f-result" });
  const text = h("div", { class: "f-text", text: findingText(f) });
  const row = h(
    "div",
    { class: `finding ${f.severity}` },
    h("i", { class: "sev" }),
    text,
  );
  if (!f.fixable) return row;

  const ask = h(
    "div",
    { class: "f-ask", style: "display:none" },
    h("span", { text: t("health.fixAsk", f.subject) }),
    h("button", {
      class: "primary",
      text: t("health.fixYes"),
      onclick: () => {
        ask.style.display = "none";
        result.textContent = "…";
        void Bridge.healthFix(f.action, f.arg)
          .then((message) => {
            result.textContent = message || t("health.fixDone", f.subject);
          })
          .catch((e: unknown) => {
            result.textContent = t("health.fixFailed", e instanceof Error ? e.message : String(e));
          });
      },
    }),
    h("button", {
      class: "ghost",
      text: t("health.cancel"),
      onclick: () => {
        ask.style.display = "none";
      },
    }),
  );

  row.append(
    h("button", {
      class: "ghost",
      text: t("health.fixClose"),
      onclick: () => {
        ask.style.display = ask.style.display === "none" ? "flex" : "none";
      },
    }),
  );
  return h("div", {}, row, ask, result);
}

function paint() {
  clear(root);

  const rescan = h("button", {
    class: "primary",
    text: busy ? t("health.scanning") : t("health.rescan"),
    onclick: () => void scan(false),
  }) as HTMLButtonElement;
  const deeper = h("button", {
    class: "ghost",
    text: t("health.rescanDeep"),
    onclick: () => void scan(true),
  }) as HTMLButtonElement;
  rescan.disabled = busy;
  deeper.disabled = busy;

  const title = report ? t("health.score", report.score) : t("health.title");
  const sub = report
    ? t("health.winSub", report.atText, report.samples, (report.tookMs / 1000).toFixed(1))
    : t("health.none");
  root.append(
    h(
      "header",
      {},
      h("div", { class: "grow" }, h("h1", { text: title }), h("div", { class: "sub", text: sub })),
      rescan,
      deeper,
    ),
  );

  if (!report) return;

  const crit = report.findings.filter((f) => f.severity === "crit").length;
  const warn = report.findings.filter((f) => f.severity === "warn").length;
  const info = report.findings.length - crit - warn;
  root.append(
    h(
      "section",
      {},
      h(
        "div",
        { class: "score-row" },
        h("div", { class: "score", text: `${report.score}%`, style: `color:${scoreColor(report.score)}` }),
        h(
          "div",
          { class: "score-meta" },
          h("div", { class: "line", text: t("health.summary", crit, warn, info) }),
          h("div", {
            class: "line",
            text: t(
              "health.loadLine",
              Math.round(report.cpuAvg),
              Math.round(report.cpuMax),
              Math.round(report.memMax),
            ),
          }),
          report.hog
            ? h("div", {
                class: "line",
                text: t("health.f.hog", report.hog, Math.round(report.hogPct)),
              })
            : "",
        ),
      ),
    ),
  );

  // A deep scan can carry the model's reading of its own findings; it goes above the list, because
  // that is the paragraph a person reads first.
  if (report.advice) {
    root.append(
      h(
        "section",
        {},
        h("h2", { text: t("health.adviceTitle") }),
        h("div", { class: "advice", text: report.advice }),
      ),
    );
  }

  const body = report.findings.length
    ? report.findings.map(findingRow)
    : [h("div", { class: "line", text: t("health.noFindings") })];
  root.append(h("section", {}, ...body));

  root.append(
    h(
      "section",
      {},
      h("h2", { text: t("health.checks") }),
      h(
        "div",
        { class: "checks" },
        ...report.checks.map((c) =>
          h(
            "div",
            { class: `check ${c.state}` },
            h("i", { class: "dot" }),
            h("strong", { text: checkName(c.id) }),
            h("span", { text: checkWord(c.state) }),
          ),
        ),
      ),
    ),
  );
}

/** One scan, on demand. The window stays live while it runs — the work is on a worker thread on
 *  the Rust side — and the button says so rather than looking dead for three seconds. */
async function scan(deep: boolean) {
  if (busy) return;
  busy = true;
  paint();
  try {
    report = await Bridge.healthScan(deep);
    void Bridge.log(`health window scan: ${report.score}% ${report.findings.length} findings`);
  } catch (e) {
    void Bridge.log(`health window scan failed: ${e instanceof Error ? e.message : String(e)}`);
  } finally {
    busy = false;
    paint();
  }
}

async function boot() {
  const info = await Bridge.boot();
  if (info) setLang(info.settings.language);
  report = await Bridge.healthLast();
  paint();
  // The window lives on between visits, so re-read the verdict when it is shown again — the island
  // may have run a scan in the meantime, and a stale window would argue with the card.
  await onEvent("pc-shown", () => {
    void Bridge.healthLast().then((fresh) => {
      report = fresh;
      paint();
    });
  });
}

void boot();
