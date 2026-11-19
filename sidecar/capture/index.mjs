// mwa capture sidecar.
// stdin: one job JSON. stdout: one JSON event per line ({type:"shot"|"done"|"fatal", ...}).
// Shared-stack safety (F0): every non-GET request to /api/* is aborted unless allow-listed,
// and each blocked request is reported with the shot. Demo responses (job.mocks, D-028) are
// answered inside the browser and never reach the server.
import { chromium } from "playwright";

const emit = (ev) => process.stdout.write(JSON.stringify(ev) + "\n");

async function readJob() {
  let text = "";
  for await (const chunk of process.stdin) text += chunk;
  return JSON.parse(text);
}

// Also hides the Next.js dev-mode badge (`next dev` on the shared stack) so it never lands in a manual.
const DISABLE_MOTION = `*,*::before,*::after{animation-duration:0s!important;animation-delay:0s!important;
transition:none!important;caret-color:transparent!important;scroll-behavior:auto!important}
nextjs-portal{display:none!important}`;

/** Resolves once no API request (SSE excluded) has been in flight for `quietMs`. */
function apiIdle(page, quietMs, timeoutMs) {
  const inflight = new Set();
  const isStream = (r) => r.resourceType() === "eventsource" || /\/stream(\?|$)/.test(r.url());
  const tracked = (r) => r.url().includes("/api/") && !isStream(r);
  const onReq = (r) => tracked(r) && inflight.add(r);
  const onEnd = (r) => inflight.delete(r);
  page.on("request", onReq);
  page.on("requestfinished", onEnd);
  page.on("requestfailed", onEnd);
  return {
    wait: () =>
      new Promise((resolve) => {
        const start = Date.now();
        let quietSince = Date.now();
        const tick = setInterval(() => {
          if (inflight.size) quietSince = Date.now();
          const timedOut = Date.now() - start > timeoutMs;
          if (Date.now() - quietSince >= quietMs || timedOut) {
            clearInterval(tick);
            resolve({ timedOut, pending: [...inflight].map((r) => r.url()) });
          }
        }, 100);
      }),
    stop: () => {
      page.off("request", onReq);
      page.off("requestfinished", onEnd);
      page.off("requestfailed", onEnd);
    },
  };
}

// Inside an open dialog we never press buttons that would commit something (F6/D-028).
const COMMIT = /(저장|생성|등록|삭제|확인|적용|실행|제출|승인|반려|업로드|완료)/;
const NAVIGATE = /(다음|이전|취소|닫기|미리\s*보기|뒤로)/;

const wordsInOrder = (text) =>
  new RegExp(text.trim().split(/\s+/).map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join(".*"));

/** Open dialogs, including confirmation boxes (role=alertdialog). */
const dialogs = (page) => page.getByRole("dialog").or(page.getByRole("alertdialog"));

const ROLE_KO = {
  button: "버튼", link: "링크", tab: "탭", row: "행", cell: "칸", checkbox: "체크박스", textbox: "입력칸",
  combobox: "선택 상자", option: "선택 항목", dialog: "창", alertdialog: "확인 창", heading: "제목", menuitem: "메뉴 항목",
};

/** Plain-language reason for a failed step (shown to the user). */
function explain(step, err) {
  const m = String(err?.message ?? err);
  const what = `${step.name ? `'${step.name}' ` : ""}${ROLE_KO[step.role] ?? step.role}`;
  let n;
  if ((n = m.match(/일치 (\d+)개/))) {
    return Number(n[1]) === 0 ? `${what}을(를) 화면에서 찾지 못함` : `같은 ${what}이(가) ${n[1]}개라 하나를 고를 수 없음`;
  }
  if (/누르지 않습니다/.test(m)) return m;
  if (/intercepts pointer events|not visible|outside of the viewport/.test(m)) return `${what}이(가) 가려져 있거나 보이지 않아 누를 수 없음`;
  if (/not enabled|disabled/.test(m)) return `${what}이(가) 비활성 상태라 누를 수 없음`;
  if ((n = m.match(/Timeout (\d+)ms/))) {
    const sec = Math.round(Number(n[1]) / 1000);
    return step.action === "wait" ? `${sec}초 기다려도 ${what}이(가) 나타나지 않음` : `${sec}초 안에 ${what} 동작을 끝내지 못함`;
  }
  return m.split("\n")[0];
}

/** Exactly one element for a planned step, or an error explaining what matched. */
async function locate(page, step) {
  let scope = page;
  // Side drawers are often not role=dialog; without an open dialog, search the whole page.
  if (step.within === "dialog" && (await dialogs(page).count()) > 0) scope = dialogs(page).last();
  // Row text joins cells without reliable spacing: match the words in order.
  if (step.row) scope = scope.getByRole("row").filter({ hasText: wordsInOrder(step.row) });
  const target = (s) =>
    s.getByRole(step.role, step.name ? { name: step.name, exact: !!step.exact } : {});
  // `near`: the innermost block that contains both the text and the target (e.g. one card).
  if (step.near) {
    scope = scope
      .locator("section, article, li, div")
      .filter({ hasText: step.near })
      .filter({ has: target(page) })
      .last();
  }
  const loc = target(scope);
  const n = await loc.count();
  if (n !== 1) {
    const where = [step.within, step.row && `행 "${step.row}"`, step.near && `"${step.near}" 근처`]
      .filter(Boolean)
      .join(", ");
    throw new Error(`${step.role} "${step.name}"${where ? ` (${where})` : ""} 일치 ${n}개`);
  }
  return loc;
}

async function runStep(page, step, idle) {
  switch (step.action) {
    case "click": {
      const loc = await locate(page, step);
      const dialogOpen = (await dialogs(page).count()) > 0;
      if (dialogOpen && COMMIT.test(step.name) && !NAVIGATE.test(step.name)) {
        throw new Error(`대화상자 안의 실행 버튼 "${step.name}"은 누르지 않습니다`);
      }
      // Styled checkboxes hide the real input (sr-only); check it directly.
      if (["checkbox", "radio", "switch"].includes(step.role)) await loc.check({ force: true, timeout: 5000 });
      else await loc.click({ timeout: 5000 });
      break;
    }
    case "fill":
      await (await locate(page, step)).fill(step.value ?? "", { timeout: 5000 });
      break;
    case "select":
      await (await locate(page, step)).selectOption(step.value ?? "", { timeout: 5000 });
      break;
    case "hover":
      await (await locate(page, step)).hover({ timeout: 5000 });
      break;
    case "wait": {
      let scope = page;
      if (step.within === "dialog" && (await dialogs(page).count()) > 0) scope = dialogs(page).last();
      const opts = { name: step.name || undefined, exact: !!step.exact };
      let target = scope.getByRole(step.role, opts);
      if (step.role === "dialog") target = target.or(scope.getByRole("alertdialog", opts));
      await target.first().waitFor({ state: "visible", timeout: 10000 });
      break;
    }
    default:
      throw new Error(`알 수 없는 동작: ${step.action}`);
  }
  await idle.wait();
  await page.waitForTimeout(300);
}

/**
 * Area to cut out, in page coordinates: the smallest element holding every `region` text (an old
 * manual's crop), else the open dialog; widened to `ratio` (old picture's width / height).
 */
async function cutBox(page, shot) {
  let box = null;
  let found = false;
  if (shot.region?.length) {
    // A picture slot of a new manual (no old picture to match) shows a whole dialog or drawer.
    const whole = !shot.ratio;
    box = await page.evaluate(({ texts, whole }) => {
      const norm = (t) => (t || "").replace(/\s+/g, "");
      const want = texts.map(norm).filter(Boolean);
      const shown = (el) => {
        const r = el.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
      };
      // Input hints count as text: a search box shows its placeholder, not innerText.
      const textOf = (el) =>
        norm(el.innerText) +
        Array.from(el.matches("input, textarea") ? [el] : el.querySelectorAll("input, textarea"))
          .map((i) => norm(i.placeholder) + norm(i.value))
          .join("");
      const holds = (el, t) => textOf(el).includes(t);
      const all = Array.from(document.querySelectorAll("body *")).filter(shown);
      // Deepest elements holding the first text, then up until every text is inside.
      const starts = all.filter((el) => holds(el, want[0]) && !Array.from(el.children).some((c) => holds(c, want[0])));
      let best = null;
      for (const st of starts) {
        let el = st;
        while (el && !want.every((t) => holds(el, t))) el = el.parentElement;
        if (!el || el === document.body) continue;
        if (whole) el = el.closest('[role="dialog"], [role="alertdialog"]') ?? el;
        const r = el.getBoundingClientRect();
        if (!best || r.width * r.height < best.width * best.height) {
          best = { x: r.x + window.scrollX, y: r.y + window.scrollY, width: r.width, height: r.height };
        }
      }
      return best;
    }, { texts: shot.region, whole });
    found = !!box;
  }
  if (!box && shot.capture === "dialog") {
    const b = await dialogs(page).last().boundingBox();
    const scroll = await page.evaluate(() => ({ x: window.scrollX, y: window.scrollY }));
    if (b) box = { x: b.x + scroll.x, y: b.y + scroll.y, width: b.width, height: b.height };
  }
  if (!box && shot.ratio && shot.capture === "page") {
    // Whole screen in the old picture: same proportions from the top.
    const vp = page.viewportSize();
    const scroll = await page.evaluate(() => ({ x: window.scrollX, y: window.scrollY }));
    return {
      box: { x: scroll.x, y: scroll.y, width: vp.width, height: Math.min(vp.height, vp.width / shot.ratio) },
      found: false,
    };
  }
  if (!box) return null;
  const doc = await page.evaluate(() => ({
    w: document.documentElement.scrollWidth,
    h: Math.max(document.documentElement.scrollHeight, window.innerHeight),
  }));
  // A picture slot is cut at the dialog's own edge; an old manual's picture had some margin.
  const pad = shot.ratio ? 8 : 1;
  let { x, y, width: w, height: h } = box;
  x -= pad;
  y -= pad;
  w += pad * 2;
  h += pad * 2;
  if (shot.ratio) {
    if (w / h < shot.ratio) {
      const nw = Math.min(h * shot.ratio, doc.w);
      x -= (nw - w) / 2;
      w = nw;
    } else {
      const nh = Math.min(w / shot.ratio, doc.h);
      y -= (nh - h) / 2;
      h = nh;
    }
  }
  x = Math.max(0, Math.min(x, doc.w - w));
  y = Math.max(0, Math.min(y, doc.h - h));
  return { box: { x, y, width: Math.min(w, doc.w), height: Math.min(h, doc.h) }, found };
}

// Open overlays first: they render last in the DOM and would be cut from a long page snapshot.
const OVERLAYS = '[role="dialog"], [role="alertdialog"], [role="complementary"], aside, [data-state="open"]';
const ariaOf = async (page) => {
  const parts = [];
  const overlays = page.locator(OVERLAYS);
  const n = Math.min(await overlays.count(), 4);
  for (let i = 0; i < n; i++) {
    const o = overlays.nth(i);
    if (!(await o.isVisible().catch(() => false))) continue;
    parts.push("# 열린 창 " + (i + 1) + "\n" + (await o.ariaSnapshot().catch(() => "")).slice(0, 12000));
  }
  const body = (await page.locator("body").ariaSnapshot()).slice(0, 30000 - parts.join("").length);
  return [...parts, "# 전체 화면\n" + body].join("\n\n");
};

/** Most specific demo response for a request, or undefined. */
function findMock(mocks, method, url) {
  const segs = url.pathname.split("/");
  let best;
  let bestScore = -1;
  for (const m of mocks ?? []) {
    if (m.method !== method) continue;
    const tpl = m.path.split("/");
    if (tpl.length !== segs.length) continue;
    const values = [];
    let ok = true;
    for (let i = 0; i < tpl.length && ok; i++) {
      if (tpl[i] === "{}") values.push(decodeURIComponent(segs[i]));
      else ok = tpl[i] === segs[i];
    }
    if (!ok) continue;
    if (m.params.length && m.params.some((p, i) => p !== values[i])) continue;
    const q = Object.entries(m.query ?? {});
    if (q.some(([k, v]) => url.searchParams.get(k) !== v)) continue;
    const score = (m.params.length ? 100 : 0) + q.length;
    if (score > bestScore) [best, bestScore] = [m, score];
  }
  return best;
}

async function main() {
  const job = await readJob();
  const base = new URL(job.baseUrl);
  const browser = await chromium.launch();
  try {
    const context = await browser.newContext({
      viewport: { width: job.viewport[0], height: job.viewport[1] },
      deviceScaleFactor: job.scale,
      locale: "ko-KR",
      timezoneId: "Asia/Seoul",
      colorScheme: "light",
    });
    await context.addCookies([
      { name: job.cookie.name, value: job.cookie.value, domain: base.hostname, path: "/", httpOnly: true },
    ]);
    if (job.domain && job.domainStorageKey) {
      await context.addInitScript(([key, db]) => sessionStorage.setItem(key, db), [job.domainStorageKey, job.domain]);
    }
    await context.addInitScript((css) => {
      const add = () => {
        const s = document.createElement("style");
        s.textContent = css;
        document.documentElement.appendChild(s);
      };
      if (document.documentElement) add();
      else document.addEventListener("DOMContentLoaded", add);
    }, DISABLE_MOTION);

    for (const shot of job.shots) {
      const page = await context.newPage();
      await page.clock.setFixedTime(new Date(job.fixedTime));
      const blocked = [];
      const mocked = [];
      const consoleErrors = [];
      page.on("console", (m) => m.type() === "error" && consoleErrors.push(m.text().slice(0, 300)));
      page.on("pageerror", (e) => consoleErrors.push(String(e).slice(0, 300)));
      await page.route("**/api/**", (route) => {
        const req = route.request();
        const url = new URL(req.url());
        const path = url.pathname;
        const mock = findMock(job.mocks, req.method(), url);
        if (mock) {
          mocked.push(`${req.method()} ${path}`);
          return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(mock.body) });
        }
        if (req.method() === "GET" || job.allowNonGet.includes(path)) return route.continue();
        blocked.push({ method: req.method(), url: path });
        return route.abort("blockedbyclient");
      });
      const idle = apiIdle(page, job.settleMs, job.timeoutMs);
      // GET /api/* bodies seen while probing: tells the planner what data really exists.
      const apiData = [];
      if (shot.aria) {
        page.on("response", async (r) => {
          try {
            const u = new URL(r.url());
            if (r.request().method() !== "GET" || !u.pathname.startsWith("/api/") || u.pathname.startsWith("/api/auth")) return;
            if (!(r.headers()["content-type"] ?? "").includes("json")) return;
            const body = await r.text();
            apiData.push({ path: u.pathname + u.search, body: body.slice(0, 1500) });
          } catch {}
        });
      }
      const started = Date.now();
      const result = { type: "shot", id: shot.id, out: shot.out, blocked, mocked, consoleErrors };
      try {
        await page.goto(new URL(shot.path, base).toString(), { waitUntil: "load", timeout: job.timeoutMs });
        const { timedOut, pending } = await idle.wait();
        await page.evaluate(() => document.fonts.ready);
        const finalPath = new URL(page.url()).pathname;
        result.finalPath = finalPath;
        result.idleTimedOut = timedOut;
        if (timedOut) result.pending = pending;
        if (finalPath.startsWith("/login")) {
          result.ok = false;
          result.error = "로그인 화면으로 이동됨 (세션 없음 또는 만료)";
        } else {
          result.stepsLog = [];
          for (const [i, step] of (shot.steps ?? []).entries()) {
            const t0 = Date.now();
            try {
              await runStep(page, step, idle);
              result.stepsLog.push({ ok: true, ms: Date.now() - t0 });
            } catch (e) {
              const plain = explain(step, e);
              result.stepsLog.push({ ok: false, ms: Date.now() - t0, error: plain });
              result.failedStep = i;
              result.aria = await ariaOf(page).catch(() => undefined);
              // What the screen looked like when it got stuck.
              const failOut = shot.out.replace(/\.png$/, ".fail.png");
              if (await page.screenshot({ path: failOut, animations: "disabled" }).then(() => true).catch(() => false)) {
                result.failShot = failOut;
              }
              throw new Error(`단계 ${i + 1}: ${plain}`);
            }
          }
          if (shot.aria) {
            result.aria = await ariaOf(page);
            // Planning needs what really exists: open each tab and keep the GET data it loads.
            const tabs = page.getByRole("tab");
            const count = Math.min(await tabs.count(), 6);
            for (let t = 1; t < count; t++) {
              const tab = tabs.nth(t);
              const label = (await tab.innerText().catch(() => "")).trim();
              if (!(await tab.isVisible().catch(() => false))) continue;
              await tab.click({ timeout: 5000 }).catch(() => {});
              await idle.wait();
              await page.waitForTimeout(300);
              result.aria += `\n\n# 탭 '${label}'을 연 화면\n` + (await ariaOf(page)).slice(0, 12000);
            }
            await tabs.first().click({ timeout: 5000 }).catch(() => {});
            result.apiData = apiData.slice(0, 40);
          }
          // Notices on screen at capture time: an error toast makes the image unusable.
          result.toasts = job.toastSelector
            ? await page
                .locator(job.toastSelector)
                .evaluateAll(
                  (els, attr) => els.map((e) => `${(attr && e.getAttribute(attr)) || "info"}: ${e.innerText.trim()}`),
                  job.toastTypeAttr,
                )
            : [];
          const mask = (job.maskButtons ?? []).map((name) => page.getByRole("button", { name }));
          const vp = page.viewportSize();
          if (shot.capture === "dialog") {
            // Tall enough that a max-height modal shows its whole body (messages at the bottom).
            await page.setViewportSize({ width: vp.width, height: Math.max(vp.height, 1600) });
            await page.waitForTimeout(400);
          }
          const clip = await cutBox(page, shot);
          if (shot.region?.length && !clip?.found) result.regionMissed = true;
          if (clip) {
            await page.screenshot({ path: shot.out, clip: clip.box, fullPage: true, mask, maskColor: "#c9ced6", animations: "disabled" });
          } else if (shot.capture === "dialog") {
            await dialogs(page).last().screenshot({ path: shot.out, animations: "disabled", timeout: 10000 });
          } else {
            const fullPage = shot.capture === "full";
            await page.screenshot({ path: shot.out, fullPage, mask, maskColor: "#c9ced6", animations: "disabled" });
          }
          if (shot.capture === "dialog") await page.setViewportSize(vp);
          result.ok = true;
        }
      } catch (e) {
        result.ok = false;
        result.error = String(e).split("\n")[0].slice(0, 300);
      } finally {
        idle.stop();
        result.ms = Date.now() - started;
        await page.close();
      }
      emit(result);
    }
  } finally {
    await browser.close();
  }
  emit({ type: "done" });
}

main().catch((e) => {
  emit({ type: "fatal", error: String(e).slice(0, 500) });
  process.exit(1);
});
