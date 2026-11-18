import { chromium } from "playwright";
// Usage: start `mwa serve`, then `npm run e2e -- <out-dir>` (makes one real chat call).
const base = process.env.MWA_URL ?? "http://127.0.0.1:7700";
const out = process.argv[2] ?? ".";
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1360, height: 860 }, colorScheme: "dark" });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
await page.goto(base);
await page.getByText("서버 연결됨").waitFor({ timeout: 10000 });
await page.getByRole("button", { name: /^연결 관리/ }).waitFor();
await page.screenshot({ path: `${out}/1-overview.png` });
// click a feature → detail, switch tabs
await page.getByRole("button", { name: /^연결 관리/ }).click();
await page.getByRole("heading", { name: "연결 관리" }).waitFor();
await page.getByRole("tab", { name: "화면·API" }).click();
await page.getByText("/api/source/systems/test").waitFor();
await page.screenshot({ path: `${out}/2-feature-apis.png` });
await page.getByRole("tab", { name: "스크린샷" }).click();
await page.locator(".shot-img").waitFor({ timeout: 10000 });
const imgOk = await page.locator(".shot-img").evaluate((img) => img.complete && img.naturalWidth > 0);
if (!imgOk) throw new Error("screenshot image did not load");
await page.screenshot({ path: `${out}/2b-shot.png` });
await page.getByRole("tab", { name: "권한" }).click();
await page.getByText("화면에서만").first().waitFor();
// slash palette
const box = page.locator("textarea");
await box.fill("/d");
await page.getByRole("option", { name: /\/doctor/ }).waitFor();
await page.screenshot({ path: `${out}/3-palette.png` });
await box.fill("");
// real chat turn about the open feature
await box.fill("이 화면에서 할 수 있는 일을 두 문장으로 설명해줘");
await box.press("Enter");
await page.waitForFunction(() => !document.querySelector(".cursor"), null, { timeout: 90000 });
const reply = await page.locator(".line.assistant .text").last().innerText();
await page.screenshot({ path: `${out}/4-chat.png` });
// model picker
await page.locator(".model-pick select").selectOption("haiku");
await page.getByText("대화 모델을 haiku").waitFor();
// command over websocket
await box.fill("/auth");
await box.press("Enter");
await page.getByText("로그인 프로파일").waitFor({ timeout: 10000 });
const status = await page.locator(".statusbar").innerText();
console.log(JSON.stringify({ reply, status, errors }, null, 1));
await browser.close();
