// mwa capture sidecar.
// stdin: one job JSON. stdout: one JSON event per line ({type:"shot"|"done"|"fatal", ...}).
// Explore mode (job.explore): one page stays open and further stdin lines are commands
// ({cmd:"open"|"step"|"shot"|"end"}), each answered with {type:"obs"} (D-057).
// Shared-stack safety (F0): every non-GET request to /api/* is aborted unless allow-listed,
// and each blocked request is reported with the shot. Demo responses (job.mocks, D-028) are
// answered inside the browser and never reach the server.
import { writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { chromium } from "playwright";

const emit = (ev) => process.stdout.write(JSON.stringify(ev) + "\n");

// The job is the first line; in explore mode the commands follow.
const stdinLines = createInterface({ input: process.stdin, crlfDelay: Infinity })[Symbol.asyncIterator]();
async function readJob() {
  const first = await stdinLines.next();
  return JSON.parse(first.value ?? "{}");
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
  text: "글자",
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
  // role "text": visible text that is not a control (a paragraph a person clicks to edit it).
  const target = (s) =>
    step.role === "text"
      ? s.getByText(step.name, { exact: !!step.exact })
      : s.getByRole(step.role, step.name ? { name: step.name, exact: !!step.exact } : {});
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
    case "press":
      // A key on the focused element (Escape closes a dropdown, Enter adds a tag).
      await page.keyboard.press(step.value || "Enter");
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
 * Lets the page grow to its whole content before a picture. App shells keep the header and the
 * menu fixed and scroll only the body inside a fixed-height box, which would cut whatever lies
 * below the fold out of every picture.
 */
async function unclip(page) {
  await page.evaluate(() => {
    const grown = new Set();
    for (const el of document.querySelectorAll("body *")) {
      const cs = getComputedStyle(el);
      if (!/(auto|scroll)/.test(cs.overflowY) || el.scrollHeight <= el.clientHeight + 4) continue;
      // A dialog keeps its place on screen; its body grows inside it.
      for (let a = el; a && a !== document.body; a = a.parentElement) {
        if (grown.has(a)) break;
        grown.add(a);
        if (getComputedStyle(a).position === "fixed" && a !== el) break;
        a.style.setProperty("overflow-y", "visible", "important");
        a.style.setProperty("height", "auto", "important");
        a.style.setProperty("max-height", "none", "important");
      }
    }
    for (const el of [document.documentElement, document.body]) {
      el.style.setProperty("height", "auto", "important");
      el.style.setProperty("overflow", "visible", "important");
    }
  });
  await page.waitForTimeout(200);
}

/**
 * Area to cut out, in page coordinates, so the picture shows the same part of the screen as the
 * old one even when that part grew: the element holding every `region` text (a whole dialog when
 * the texts fill most of it; otherwise the enclosing block shaped most like the old picture),
 * else the open dialog, else the page's body column. Never the app's header or side menu, and
 * nothing is cut: the old proportions only choose among whole blocks.
 */
async function cutBox(page, shot) {
  const box = await page.evaluate(({ texts, ratio, capture }) => {
    const norm = (t) => (t || "").replace(/\s+/g, "");
    const shown = (el) => {
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0;
    };
    const rect = (el) => {
      const r = el.getBoundingClientRect();
      return { x: r.x + window.scrollX, y: r.y + window.scrollY, width: r.width, height: r.height };
    };
    const DIALOG = '[role="dialog"], [role="alertdialog"]';
    // The app's top bar and side menu, found by where they sit (many apps mark them with plain
    // divs): a low full-width band at the top, a narrow full-height band at the left.
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const band = (x, y, fits) => {
      let el = document.elementFromPoint(x, y);
      let out = null;
      while (el && el !== document.body && el !== document.documentElement) {
        const r = el.getBoundingClientRect();
        if (!fits(r)) break;
        out = r;
        el = el.parentElement;
      }
      return out;
    };
    const top = band(vw / 2, 2, (r) => r.height < 160);
    const side = band(2, Math.min(vh - 2, (top?.bottom ?? 0) + vh / 2), (r) => r.width < 320);
    const topEdge = top && top.width > vw * 0.8 ? top.bottom + window.scrollY : 0;
    const leftEdge = side && side.height > vh * 0.5 ? side.right + window.scrollX : 0;
    // Growing past this would take in the app's side menu or top bar.
    const tooFar = (from, a) => {
      const r = rect(a);
      return a === document.body || r.x < leftEdge - 2 || r.y < topEdge - 2;
    };
    const off = (r) => (ratio ? Math.abs(Math.log(r.width / r.height / ratio)) : 0);
    // The enclosing block shaped most like the old picture, up to `limit` (a dialog) at most.
    const shape = (el, limit) => {
      const base = rect(el);
      let pick = el;
      let best = rect(el);
      for (let a = el.parentElement, n = 0; a && n < 10 && !(limit ? !limit.contains(a) : tooFar(base, a)); a = a.parentElement) {
        const ra = rect(a);
        if (ra.width * ra.height <= best.width * best.height * 1.02) continue;
        n++;
        if (ratio && off(ra) < off(best) - 0.05) {
          pick = a;
          best = ra;
        }
      }
      return pick;
    };

    const want = (texts || []).map(norm).filter(Boolean);
    if (want.length) {
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
        const dialog = el.closest(DIALOG);
        let pick;
        if (dialog) {
          const rd = rect(dialog);
          const re = rect(el);
          // Most of the dialog in the old picture: the whole dialog, title and buttons too.
          pick = !ratio || re.width * re.height >= rd.width * rd.height * 0.5 ? dialog : shape(el, dialog);
        } else {
          pick = ratio ? shape(el, null) : el;
        }
        const r = rect(pick);
        if (!best || r.width * r.height < best.width * best.height) best = { ...r, found: true, topEdge };
      }
      if (best) return best;
    }
    if (capture === "dialog") {
      const ds = Array.from(document.querySelectorAll(DIALOG)).filter(shown);
      if (ds.length) return { ...rect(ds[ds.length - 1]), found: false, topEdge: 0 };
    }
    if (capture === "page" && ratio) {
      // The whole screen in the old picture: the body column, the largest block clear of the top
      // bar and the side menu.
      let el = document.elementFromPoint((leftEdge + vw) / 2, Math.min(vh - 2, topEdge + 48));
      if (!el) return null;
      const from = rect(el);
      while (el.parentElement && !tooFar(from, el.parentElement)) el = el.parentElement;
      const r = rect(el);
      // An old picture of one screenful stays one screenful: a body column grown far below the
      // fold (a side panel unrolled) is cut where the old picture ended.
      if (r.height > (r.width / ratio) * 1.3) r.height = r.width / ratio;
      return { ...r, found: false, topEdge };
    }
    return null;
  }, { texts: shot.region ?? [], ratio: shot.ratio ?? 0, capture: shot.capture });
  if (!box) return null;
  const doc = await page.evaluate(() => ({
    w: document.documentElement.scrollWidth,
    h: Math.max(document.documentElement.scrollHeight, window.innerHeight),
  }));
  // A picture slot is cut at the dialog's own edge; an old manual's picture had some margin.
  const pad = shot.ratio ? 8 : 1;
  let { x, y, width: w, height: h } = box;
  x = Math.max(0, x - pad);
  // The margin stops at the app's top bar (its shadow would show as a dark line).
  y = Math.max(y >= box.topEdge ? box.topEdge : 0, y - pad);
  w = Math.min(w + pad * 2, doc.w - x);
  h = Math.min(h + pad * 2, doc.h - y);
  return { box: { x, y, width: w, height: h }, found: box.found };
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

/** A fresh page whose /api/* writes are blocked (or answered by demo data). */
async function openPage(context, job) {
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
  return { page, blocked, mocked, consoleErrors };
}

/** Cuts and saves the picture of the screen as it is now; notes toasts and a missed region. */
async function takePicture(page, shot, job, result) {
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
  if (shot.capture !== "full") await unclip(page);
  // A dialog unrolled taller than the window is centred off its top: grow the window to it.
  const d = dialogs(page).last();
  const db = (await d.count()) > 0 ? await d.boundingBox().catch(() => null) : null;
  const now = page.viewportSize();
  if (db && (db.y < 0 || db.y + db.height > now.height)) {
    await page.setViewportSize({ width: now.width, height: Math.min(Math.ceil(db.height) + 120, 8000) });
    await page.waitForTimeout(400);
  }
  const clip = await cutBox(page, shot);
  if (shot.region?.length && !clip?.found) result.regionMissed = true;
  if (clip) {
    await page.screenshot({ path: shot.out, clip: clip.box, fullPage: true, mask, maskColor: "#c9ced6", animations: "disabled" });
    // The whole screen and where the picture was cut from it, so a person can cut again.
    const full = shot.out.replace(/\.png$/, ".full.png");
    if (await page.screenshot({ path: full, fullPage: true, mask, maskColor: "#c9ced6", animations: "disabled" }).then(() => true).catch(() => false)) {
      const k = job.scale || 1;
      const b = clip.box;
      writeFileSync(shot.out.replace(/\.png$/, ".box.json"), JSON.stringify({ x: Math.round(b.x * k), y: Math.round(b.y * k), width: Math.round(b.width * k), height: Math.round(b.height * k) }));
    }
  } else if (shot.capture === "dialog") {
    await dialogs(page).last().screenshot({ path: shot.out, animations: "disabled", timeout: 10000 });
  } else {
    const fullPage = shot.capture === "full";
    await page.screenshot({ path: shot.out, fullPage, mask, maskColor: "#c9ced6", animations: "disabled" });
  }
  if (shot.capture === "dialog") await page.setViewportSize(vp);
}

/**
 * Explore mode: one page, driven one command at a time by a model that looks at each screen
 * (D-057). Answers every command with what the screen shows now: its accessibility tree and,
 * when `look` names a file, a screenshot of the viewport.
 */
async function explore(context, job, base) {
  const { page, blocked } = await openPage(context, job);
  const idle = apiIdle(page, job.settleMs, job.timeoutMs);
  try {
    for (;;) {
      const next = await stdinLines.next();
      if (next.done) break;
      const c = JSON.parse(next.value);
      if (c.cmd === "end") break;
      const obs = { type: "obs", ok: true };
      try {
        if (c.cmd === "open") {
          await page.goto(new URL(c.path, base).toString(), { waitUntil: "load", timeout: job.timeoutMs });
          await idle.wait();
          await page.evaluate(() => document.fonts.ready);
          if (new URL(page.url()).pathname.startsWith("/login")) throw new Error("로그인 화면으로 이동됨 (세션 없음 또는 만료)");
        } else if (c.cmd === "step") {
          await runStep(page, c.step, idle);
        } else if (c.cmd === "shot") {
          // The last command of a scene: the picture unrolls the page.
          await takePicture(page, c.shot, job, obs);
          obs.out = c.shot.out;
        }
      } catch (e) {
        obs.ok = false;
        obs.error = c.step ? explain(c.step, e) : String(e).split("\n")[0].slice(0, 300);
      }
      if (c.cmd !== "shot") {
        obs.aria = await ariaOf(page).catch(() => "");
        if (c.look && (await page.screenshot({ path: c.look, animations: "disabled" }).then(() => true).catch(() => false))) obs.look = c.look;
      }
      obs.url = new URL(page.url()).pathname + new URL(page.url()).search;
      obs.blocked = blocked.splice(0);
      emit(obs);
    }
  } finally {
    idle.stop();
    await page.close();
  }
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

    if (job.explore) return await explore(context, job, base);
    for (const shot of job.shots) {
      const { page, blocked, mocked, consoleErrors } = await openPage(context, job);
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
          await takePicture(page, shot, job, result);
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
