import { useEffect, useState } from "react";
import { fetchSettings, saveSettings, type Settings } from "../api";

/** Which role each step uses (see the engine's calls). */
const STEPS: { step: string; roles: string[]; what: string; example?: string }[] = [
  { step: "① 장면 설명", roles: ["worker"], what: "기존 매뉴얼 그림을 하나씩 보고 어떤 화면·상태인지 적습니다", example: "그림 14장 $0.02" },
  { step: "② 촬영 계획·실패 수정", roles: ["writer"], what: "그림과 화면 구조를 보고 클릭 순서를 짜고, 막히면 다시 짭니다" },
  { step: "② 그림 대조", roles: ["writer"], what: "새로 찍은 그림이 기존 그림과 같은 장면인지 판정합니다", example: "장면 12개 촬영+대조 $0.15~0.9" },
  { step: "③ 기존 매뉴얼 옮기기", roles: ["worker"], what: "PDF 쪽마다 글·표·그림 위치를 편집 문서로 옮깁니다", example: "10쪽 $0.01" },
  { step: "③ 코드 다시 읽기", roles: ["worker"], what: "코드에서 사실 뽑기 (인용 줄 검사)" },
  { step: "③ 바뀐 곳 제안", roles: ["writer"], what: "문서와 코드 사실을 비교해 고칠 곳을 제안합니다", example: "제안 6개 $0.08" },
  { step: "④ 우클릭 도우미", roles: ["chat"], what: "묻기·다듬기·고쳐 쓰기" },
];

const ROLE_KO: Record<string, string> = { worker: "일꾼", writer: "작성자", verifier: "검증자", chat: "도우미" };

const MODELS: { name: string; info: string }[] = [
  { name: "haiku", info: "가장 빠르고 사용량이 적습니다. 그림 읽기·쪽 옮기기·사실 추출처럼 양이 많고 단순한 일에 맞습니다. 섬세한 판단은 약합니다." },
  { name: "sonnet", info: "속도와 품질의 균형. 촬영 계획·작성·제안처럼 판단이 필요한 대부분의 일에 맞습니다." },
  { name: "opus", info: "가장 꼼꼼하지만 느리고 사용량이 큽니다. 근거 검증처럼 틀리면 안 되는 일에 씁니다." },
  { name: "fable", info: "최신 모델입니다. 이 도구에서는 아직 다른 모델과 비교해 보지 않았습니다." },
];

const RECOMMENDED: Record<string, string> = { worker: "haiku", writer: "sonnet", verifier: "opus", chat: "sonnet" };

/** How work is split across models (project-wide), with what each model is good at. */
export default function ModelPanel() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [roles, setRoles] = useState<Settings["roles"]>([]);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    fetchSettings()
      .then((s) => {
        setSettings(s);
        setRoles(s.roles);
      })
      .catch((e) => setMsg({ ok: false, text: String(e) }));
  }, []);

  if (!settings) return msg ? <p className="flash err">{msg.text}</p> : <p className="muted">모델 구성을 불러오는 중…</p>;

  const modelOf = (role: string) => roles.find((r) => r.role === role)?.model ?? "";
  const dirty = roles.some((r) => settings.roles.find((x) => x.role === r.role)?.model !== r.model);
  const save = async () => {
    try {
      await saveSettings({ ...settings, roles });
      setSettings({ ...settings, roles });
      setMsg({ ok: true, text: "저장했습니다. 다음 작업부터 적용됩니다." });
    } catch (e) {
      setMsg({ ok: false, text: String((e as Error).message ?? e) });
    }
  };

  return (
    <div className="models">
      <p className="muted small">
        일을 네 역할로 나눠 역할마다 모델을 정합니다. 양이 많고 단순한 일은 빠른 모델, 판단이 필요한 일은 균형 모델, 틀리면 안 되는 검증은 꼼꼼한 모델에 맡깁니다. 프로젝트 전체에 적용됩니다. 사용량은 구독 한도 안에서 쓰고, 금액은 정가로 바꿔 본 값입니다(따로 청구되지 않음).
      </p>
      <table className="models-table">
        <thead>
          <tr>
            <th>역할</th>
            <th>모델</th>
            <th>맡는 단계</th>
          </tr>
        </thead>
        <tbody>
          {roles.map((r) => (
            <tr key={r.role}>
              <td>
                <b>{ROLE_KO[r.role] ?? r.role}</b> <span className="muted small mono">{r.role}</span>
              </td>
              <td>
                <select value={r.model} onChange={(e) => setRoles(roles.map((x) => (x.role === r.role ? { ...x, model: e.target.value } : x)))}>
                  {MODELS.map((m) => (
                    <option key={m.name} value={m.name}>
                      {m.name}
                      {RECOMMENDED[r.role] === m.name ? " (추천)" : ""}
                    </option>
                  ))}
                  {!MODELS.some((m) => m.name === r.model) && <option value={r.model}>{r.model}</option>}
                </select>
              </td>
              <td className="small">
                {STEPS.filter((s) => s.roles.includes(r.role)).map((s) => (
                  <div key={s.step}>
                    <b>{s.step}</b> — {s.what}
                    {s.example && <span className="muted"> · 예: {s.example}</span>}
                  </div>
                ))}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="models-info">
        {MODELS.map((m) => (
          <div key={m.name} className={`model-card ${roles.some((r) => r.model === m.name) ? "used" : ""}`}>
            <b className="mono">{m.name}</b>
            <span className="small">{m.info}</span>
            <span className="muted small">
              쓰는 역할: {roles.filter((r) => r.model === m.name).map((r) => ROLE_KO[r.role] ?? r.role).join(", ") || "없음"}
            </span>
          </div>
        ))}
      </div>
      <div className="step-actions">
        <button className="btn primary" disabled={!dirty} onClick={save}>{dirty ? "모델 구성 저장" : "저장됨"}</button>
        {modelOf("worker") && modelOf("worker") !== "haiku" && <span className="muted small">일꾼을 haiku보다 큰 모델로 두면 장면 설명·쪽 옮기기 사용량이 크게 늡니다.</span>}
        {msg && <span className={`small ${msg.ok ? "ok" : "err"}`}>{msg.text}</span>}
      </div>
    </div>
  );
}
