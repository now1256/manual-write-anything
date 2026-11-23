import { useEffect, useRef, useState } from "react";
import { fetchSettings, fetchSetup, saveSettings, setPassword, type SetupStep } from "../api";
import SettingsView from "./SettingsView";

interface Props {
  /** Bumped after commands finish, so finished steps show. */
  version: number;
  /** A command is running (its progress shows above). */
  busy: string | null;
  onCommand: (name: string, arg?: string) => void;
  onClose: () => void;
}

/** The steps a project is set up with, in order; the rest of "시작하기" is the work itself. */
const ORDER = ["code", "claude", "product", "account", "template", "shots"];
const OPTIONAL = new Set(["template", "shots"]);

const HELP: Record<string, string> = {
  code: "매뉴얼을 쓸 제품의 프론트·백엔드 코드를 연결합니다. 이 서버의 폴더, Git 주소, 또는 다른 서버(SSH)의 폴더를 쓸 수 있습니다. 코드는 읽기만 합니다.",
  claude: "글쓰기와 판단은 이 서버에 로그인된 Claude 구독으로 합니다. 이 서버의 터미널에서 한 번 로그인하면 됩니다.",
  product: "Claude가 코드를 읽고 메뉴 파일, API 호출 방식, 권한, 로그인 방식을 찾아 어댑터를 만든 뒤, 기능 목록(제품 지도)을 만들어 확인합니다.",
  account: "mwa가 화면을 찍을 사이트 주소와 로그인 계정입니다. 비밀번호는 이 서버의 자격 파일에만 저장되고, 로그인은 1번만 시도합니다.",
  template: "선택. 매뉴얼 양식을 배울 견본 매뉴얼(PDF·MD)을 지정합니다. 없으면 기존 매뉴얼과 이미 만든 매뉴얼을 본보기로 씁니다.",
  shots: "선택. 기능마다 첫 화면을 찍어 둡니다. 새 매뉴얼의 화면 그림으로 씁니다 (쓰기 요청은 막음).",
};

const RUN_LABEL: Record<string, string> = {
  sync: "소스 받기",
  adapter: "코드 읽고 제품 파악 시작",
  map: "제품 지도 만들기",
  shot: "첫 화면 촬영 시작",
};

/**
 * Setting a project up, one step at a time: the steps on the left (done, now, later), only
 * the current step's settings or command on the right, and a clear "done" before moving on.
 */
export default function SetupWizard({ version, busy, onCommand, onClose }: Props) {
  const [steps, setSteps] = useState<SetupStep[] | null>(null);
  const [current, setCurrent] = useState<string | null>(null);
  const [skipped, setSkipped] = useState<Set<string>>(new Set());
  /** A step that just finished: shown as done, with the way on. */
  const [justDone, setJustDone] = useState<SetupStep | null>(null);
  const before = useRef<Map<string, boolean>>(new Map());
  const [saved, setSaved] = useState(0);

  useEffect(() => {
    let live = true;
    fetchSetup()
      .then((all) => {
        if (!live) return;
        const mine = ORDER.map((k) => all.find((s) => s.key === k)).filter((s): s is SetupStep => !!s);
        // A step that turned done since the last look: celebrate it.
        const finished = mine.find((s) => s.done && before.current.get(s.key) === false);
        before.current = new Map(mine.map((s) => [s.key, s.done]));
        setSteps(mine);
        if (finished) setJustDone(finished);
        setCurrent((c) => c ?? mine.find((s) => !s.done)?.key ?? mine[0]?.key ?? null);
      })
      .catch(() => live && setSteps([]));
    return () => {
      live = false;
    };
  }, [version, saved]);

  if (!steps) return <p className="muted pad">불러오는 중…</p>;
  const left = steps.filter((s) => !s.done && !skipped.has(s.key));
  const required = steps.filter((s) => !OPTIONAL.has(s.key));
  const allSet = required.every((s) => s.done) && left.every((s) => OPTIONAL.has(s.key) && skipped.has(s.key));
  // Required steps left anywhere come first; then the optional ones after this step.
  const nextAfter = (key: string) => {
    const open = (s: SetupStep) => !s.done && !skipped.has(s.key) && s.key !== key;
    const i = steps.findIndex((s) => s.key === key);
    return (
      steps.find((s) => open(s) && !OPTIONAL.has(s.key)) ??
      steps.slice(i + 1).find(open) ??
      steps.find(open) ??
      null
    );
  };
  const go = (key: string | null) => {
    setJustDone(null);
    setCurrent(key);
  };
  const step = steps.find((s) => s.key === current) ?? left[0] ?? null;
  const command = step && step.action.startsWith("command:") ? step.action.slice(8) : null;
  const n = (key: string) => steps.findIndex((s) => s.key === key) + 1;

  return (
    <div className="wizard">
      <div className="wizard-head">
        <h2>프로젝트 설정</h2>
        <span className="muted small">
          {steps.filter((s) => s.done).length}/{steps.length} 완료{skipped.size ? ` · 건너뜀 ${skipped.size}` : ""}
        </span>
        <span className="spacer" />
        <button className={allSet ? "btn primary" : "link"} onClick={onClose}>{allSet ? "매뉴얼 현황으로" : "나중에 하기"}</button>
      </div>
      <div className="wizard-body">
        <ol className="wizard-steps">
          {steps.map((s) => {
            // An optional step with nothing set counts as fine, not as something done.
            const idle = OPTIONAL.has(s.key) && s.done && s.detail.startsWith("선택");
            const state = idle ? "skipped" : s.done ? "done" : skipped.has(s.key) ? "skipped" : s.key === current ? "now" : "later";
            return (
              <li key={s.key} className={`${state} ${s.key === current ? "on" : ""}`}>
                <button onClick={() => go(s.key)}>
                  <span className="mark">{idle ? "–" : s.done ? "✓" : skipped.has(s.key) ? "–" : n(s.key)}</span>
                  <span className="what">
                    <b>{s.label}</b>
                    {OPTIONAL.has(s.key) && <span className="muted small"> (선택)</span>}
                    <span className="muted small">{idle ? "없어도 됨" : s.done ? s.detail : skipped.has(s.key) ? "건너뜀" : ""}</span>
                  </span>
                </button>
              </li>
            );
          })}
        </ol>

        <div className="wizard-panel">
          {allSet && !justDone ? (
            <div className="wizard-finish">
              <div className="big-check">✓</div>
              <h3>설정이 끝났습니다</h3>
              <p className="muted">이제 매뉴얼 현황에서 기능을 골라 ① 기존 매뉴얼 → ④ 편집 순서로 매뉴얼을 만듭니다.</p>
              <button className="btn primary" onClick={onClose}>매뉴얼 현황으로</button>
            </div>
          ) : justDone ? (
            <div className="wizard-done">
              <div className="big-check">✓</div>
              <h3>{justDone.label} 완료</h3>
              <p>{justDone.detail}</p>
              {(() => {
                const next = nextAfter(justDone.key);
                return next ? (
                  <button className="btn primary" onClick={() => go(next.key)}>
                    다음: {next.label} →
                  </button>
                ) : (
                  <button className="btn primary" onClick={() => go(null)}>
                    마치기 →
                  </button>
                );
              })()}
            </div>
          ) : step ? (
            <>
              <div className="wizard-title">
                <span className="no">{step.done ? "✓" : n(step.key)}</span>
                <h3>{step.label}</h3>
                {OPTIONAL.has(step.key) && <span className="badge">선택</span>}
                {step.done && <span className="badge ok">완료</span>}
              </div>
              <p className="muted">{HELP[step.key]}</p>
              <p className={`wizard-state ${step.done ? "ok-text" : ""}`}>
                {step.done ? "✓ " : "지금 상태: "}
                {step.detail}
              </p>

              {step.key === "claude" && !step.done && (
                <div className="wizard-cmd">
                  <p className="small">이 서버의 터미널에서 실행하세요:</p>
                  <pre className="mono">claude auth login</pre>
                  <button className="btn" onClick={() => setSaved((x) => x + 1)}>로그인했으면 다시 확인</button>
                </div>
              )}

              {step.key === "account" && <AccountStep onSaved={() => setSaved((x) => x + 1)} />}

              {["code", "template"].includes(step.key) && (
                <div className="wizard-settings">
                  <SettingsView embedded focus={step.key} onSaved={() => setSaved((x) => x + 1)} onClose={() => {}} onCommand={onCommand} />
                </div>
              )}

              {command && !step.done && (
                <div className="wizard-run">
                  <button className="btn primary big" disabled={!!busy} onClick={() => onCommand(command)}>
                    {busy ? `${busy} 진행 중… (위 표시)` : (RUN_LABEL[command] ?? command)}
                  </button>
                  {step.key === "code" && <span className="muted small">설정을 저장한 뒤 누르면 코드를 가져옵니다.</span>}
                </div>
              )}
              {command && step.done && step.key !== "code" && (
                <button className="btn" disabled={!!busy} onClick={() => onCommand(command)}>
                  다시 하기
                </button>
              )}

              <div className="wizard-foot">
                {OPTIONAL.has(step.key) && !step.done && (
                  <button
                    className="btn"
                    onClick={() => {
                      setSkipped((s) => new Set(s).add(step.key));
                      go(nextAfter(step.key)?.key ?? null);
                    }}
                  >
                    건너뛰기
                  </button>
                )}
                {step.done && nextAfter(step.key) && (
                  <button className="btn primary" onClick={() => go(nextAfter(step.key)!.key)}>
                    다음: {nextAfter(step.key)!.label} →
                  </button>
                )}
              </div>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}

/**
 * The site to shoot and its login, in one go: the account, the capture settings and the
 * password are saved together. No login is tried here (it is tried once, when shooting).
 */
function AccountStep({ onSaved }: { onSaved: () => void }) {
  const [url, setUrl] = useState("");
  const [user, setUser] = useState("");
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const [name, setName] = useState("site");
  useEffect(() => {
    fetchSettings()
      .then((s) => {
        const p = s.profiles.find((x) => x.name === s.capture?.profile) ?? s.profiles[0];
        if (p) {
          setName(p.name);
          setUrl(p.base_url);
          setUser(p.user_id);
        }
      })
      .catch(() => {});
  }, []);
  const save = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const s = await fetchSettings();
      const at = s.profiles.findIndex((p) => p.name === name);
      const profile = { name, base_url: url.trim().replace(/\/$/, ""), user_id: user.trim(), purpose: "capture" };
      if (at >= 0) s.profiles[at] = profile;
      else s.profiles.push(profile);
      s.capture = s.capture ? { ...s.capture, profile: name } : { profile: name, domain: "", viewport: [1440, 900], scale: 2, node: "" };
      await saveSettings(s);
      if (pw) await setPassword(name, pw);
      setPw("");
      setMsg({ ok: true, text: "저장했습니다. 촬영할 때 이 계정으로 1번 로그인합니다." });
      onSaved();
    } catch (x) {
      setMsg({ ok: false, text: String((x as Error).message ?? x) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="wizard-form">
      <label>
        사이트 주소
        <input placeholder="http://10.0.0.5:3001" value={url} onChange={(e) => setUrl(e.target.value)} />
        <span className="muted small">mwa가 이 서버에서 열 주소입니다. 코드 연결에서 SSH로 붙였다면 그 화면의 "제품이 떠 있는 주소"에 후보가 나옵니다.</span>
      </label>
      <label>
        아이디
        <input autoComplete="off" value={user} onChange={(e) => setUser(e.target.value)} />
      </label>
      <label>
        비밀번호
        <input type="password" autoComplete="new-password" placeholder="이미 저장했으면 비워 두세요" value={pw} onChange={(e) => setPw(e.target.value)} />
      </label>
      <div className="wizard-run">
        <button className="btn primary big" disabled={busy || !url.trim() || !user.trim()} onClick={() => void save()}>
          {busy ? "저장 중…" : "저장"}
        </button>
        {msg && <span className={msg.ok ? "ok-text" : "err-text"}>{msg.ok ? "✓ " : "✗ "}{msg.text}</span>}
      </div>
    </div>
  );
}
