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

/** Exactly one element for a planned step, or an error explaining what matched. */
async function locate(page, step) {
  let scope = page;
  if (step.within === "dialog") scope = page.getByRole("dialog").last();
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
      const dialogOpen = (await page.getByRole("dialog").count()) > 0;
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
      if (step.within === "dialog") scope = page.getByRole("dialog").last();
      await scope.getByRole(step.role, { name: step.name || undefined, exact: !!step.exact }).first()
        .waitFor({ state: "visible", timeout: 10000 });
      break;
    }
    default:
      throw new Error(`알 수 없는 동작: ${step.action}`);
  }
  await idle.wait();
  await page.waitForTimeout(300);
}

const ariaOf = async (page) => (await page.locator("body").ariaSnapshot()).slice(0, 30000);

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
          for (const [i, step] of (shot.steps ?? []).entries()) {
            try {
              await runStep(page, step, idle);
            } catch (e) {
              result.failedStep = i;
              result.aria = await ariaOf(page).catch(() => undefined);
              throw new Error(`단계 ${i + 1}(${step.action} ${step.role} "${step.name}"): ${String(e.message ?? e).split("\n")[0]}`);
            }
          }
          if (shot.aria) result.aria = await ariaOf(page);
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
          if (shot.capture === "dialog") {
            await page.getByRole("dialog").last().screenshot({ path: shot.out, animations: "disabled" });
          } else {
            const fullPage = shot.capture === "full";
            await page.screenshot({ path: shot.out, fullPage, mask, maskColor: "#c9ced6", animations: "disabled" });
          }
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
