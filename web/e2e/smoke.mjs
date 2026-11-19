// Smoke test of the web screens against a running engine (no product names: works for any project).
// Usage: start `mwa serve`, then `MWA_URL=http://127.0.0.1:7000 npm run e2e -- <out-dir>`.
// Makes one real helper call (Claude) when the first feature with a manual document has one.
import { chromium } from "playwright";

const url = process.env.MWA_URL ?? "http://127.0.0.1:7000";
const out = process.argv[2] ?? ".";
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1500, height: 1000 } });
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));

await page.goto(url);
await page.getByText("서버 연결됨").waitFor({ timeout: 10000 });

// 1. Home: the three jobs and the manual status board.
await page.getByRole("heading", { name: "매뉴얼 현황" }).waitFor();
const rows = page.locator(".manuals-table tbody tr");
if ((await rows.count()) === 0) throw new Error("매뉴얼 현황에 기능이 없습니다 (제품 지도를 먼저 만드세요)");
await page.screenshot({ path: `${out}/1-home.png` });

// 2. A feature: prefer one that already has a document, else the first.
const withDoc = rows.filter({ hasText: "버전" }).first();
const row = (await withDoc.count()) ? withDoc : rows.first();
await row.click();
await page.getByRole("tab", { name: "① 기존 매뉴얼" }).waitFor();
for (const tab of ["② 그림 고르기", "③ 매뉴얼 만들기", "화면·API", "권한"]) {
  await page.getByRole("tab", { name: tab }).click();
  await page.waitForTimeout(300);
}
await page.screenshot({ path: `${out}/2-feature.png` });

// 3. Editor.
await page.getByRole("tab", { name: "④ 편집·버전" }).click();
await page.waitForTimeout(1500);
await page.screenshot({ path: `${out}/3-editor.png` });

// 4. One real helper call, when there is a document to ask about.
if (await page.locator(".doc-page .doc-body p").count()) {
  await page.locator(".doc-page .doc-body p").first().click();
  await page.locator(".req-input textarea").fill("이 문단을 한 문장으로 요약만 해 줘. 문서는 바꾸지 마.");
  await page.locator(".req-input textarea").press("Enter");
  await page.locator(".turn.assistant:not(.busy)").first().waitFor({ timeout: 180000 });
  await page.screenshot({ path: `${out}/4-helper.png` });
} else {
  console.log("문서가 있는 기능이 없어 도우미 호출은 건너뜁니다");
}

// 5. Settings page opens.
await page.getByRole("button", { name: "설정" }).click();
await page.getByRole("heading", { name: "확장 기능" }).waitFor({ timeout: 10000 });

if (errors.length) throw new Error(`화면 오류: ${errors.join(" / ")}`);
console.log(`ok → ${out}`);
await browser.close();
