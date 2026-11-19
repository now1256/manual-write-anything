import { useEffect, useState } from "react";
import { fetchAutomation, saveAutomation, type AutoRule, type Automation } from "../api";

interface Props {
  /** Features to pick from (key, label). */
  features: { key: string; label: string }[];
  /** Bumped after commands finish. */
  version: number;
  onCommand: (name: string, arg?: string) => void;
}

const WEEKDAYS = ["월", "화", "수", "목", "금", "토", "일"];

const fresh = (): AutoRule => ({
  id: "",
  name: "매일 갱신",
  enabled: true,
  every: "daily",
  at: "09:00",
  weekday: 1,
  hours: 6,
  scope: "changed",
  features: [],
  action: "propose",
  reshoot: true,
  until: "review",
});

const when = (t?: number | null) =>
  t ? new Date(t * 1000).toLocaleString("ko-KR", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }) : "—";

const STATUS: Record<string, { mark: string; label: string }> = {
  running: { mark: "…", label: "도는 중" },
  ok: { mark: "✓", label: "완료" },
  none: { mark: "–", label: "할 일 없음" },
  failed: { mark: "✗", label: "실패" },
};

/** 자동 작업: rules read as sentences, with their run history (home screen, under 매뉴얼 현황). */
export default function AutomationPanel({ features, version, onCommand }: Props) {
  const [auto, setAuto] = useState<Automation | null>(null);
  const [rules, setRules] = useState<AutoRule[]>([]);
  const [dirty, setDirty] = useState(false);
  const [err, setErr] = useState("");

  useEffect(() => {
    fetchAutomation()
      .then((a) => {
        setAuto(a);
        setRules(a.rules);
        setDirty(false);
      })
      .catch(() => setAuto({ rules: [], runs: [], running: false }));
  }, [version]);

  const change = (i: number, f: (r: AutoRule) => void) => {
    setRules((all) => all.map((r, k) => (k === i ? (() => { const c = { ...r, features: [...r.features] }; f(c); return c; })() : r)));
    setDirty(true);
  };
  const save = async (next = rules) => {
    try {
      setErr("");
      const a = await saveAutomation(next);
      setAuto(a);
      setRules(a.rules);
      setDirty(false);
    } catch (x) {
      setErr(String((x as Error).message ?? x));
    }
  };
  const label = (k: string) => features.find((f) => f.key === k)?.label ?? k;

  if (!auto) return null;
  return (
    <section className="auto">
      <div className="auto-head">
        <h2>자동 작업</h2>
        <span className="muted small">정해 둔 때에 코드를 다시 읽고 매뉴얼을 맞춥니다. 서버(mwa serve)가 떠 있는 동안 돕니다.</span>
        <span className="spacer" />
        {dirty && (
          <button className="btn primary" onClick={() => void save()}>
            저장
          </button>
        )}
        <button
          className="btn"
          onClick={() => {
            setRules((all) => [...all, fresh()]);
            setDirty(true);
          }}
        >
          + 규칙 추가
        </button>
      </div>
      {err && <p className="flash err">{err}</p>}
      {rules.length === 0 && <p className="muted small">아직 규칙이 없습니다. "+ 규칙 추가"로 예를 들어 "매일 09:00에 코드가 바뀐 기능만 제안 받기"를 만들 수 있습니다.</p>}
      {rules.map((r, i) => (
        <div key={r.id || `new-${i}`} className={`auto-rule ${r.enabled ? "" : "off"}`}>
          <div className="auto-line">
            <button className={`auto-toggle ${r.enabled ? "on" : ""}`} onClick={() => change(i, (c) => (c.enabled = !c.enabled))}>
              {r.enabled ? "● 켜짐" : "○ 꺼짐"}
            </button>
            <input className="auto-name" value={r.name} onChange={(e) => change(i, (c) => (c.name = e.target.value))} />
          </div>
          <div className="auto-sentence">
            <select value={r.every} onChange={(e) => change(i, (c) => (c.every = e.target.value as AutoRule["every"]))}>
              <option value="daily">매일</option>
              <option value="weekly">매주</option>
              <option value="hours">몇 시간마다</option>
            </select>
            {r.every === "weekly" && (
              <select value={r.weekday} onChange={(e) => change(i, (c) => (c.weekday = Number(e.target.value)))}>
                {WEEKDAYS.map((d, k) => (
                  <option key={d} value={k + 1}>{d}요일</option>
                ))}
              </select>
            )}
            {r.every === "hours" ? (
              <>
                <input type="number" min={1} className="auto-hours" value={r.hours} onChange={(e) => change(i, (c) => (c.hours = Number(e.target.value)))} />
                <span>시간마다</span>
              </>
            ) : (
              <>
                <input type="time" value={r.at} onChange={(e) => change(i, (c) => (c.at = e.target.value))} />
                <span>에</span>
              </>
            )}
            <select value={r.scope} onChange={(e) => change(i, (c) => (c.scope = e.target.value as AutoRule["scope"]))}>
              <option value="changed">코드가 바뀐 기능</option>
              <option value="features">고른 메뉴</option>
            </select>
            <span>만</span>
            <select value={r.action} onChange={(e) => change(i, (c) => (c.action = e.target.value as AutoRule["action"]))}>
              <option value="propose">코드 다시 읽고 바뀐 곳 제안</option>
              <option value="rewrite">코드로 매뉴얼 다시 쓰기</option>
            </select>
            <label className="auto-check">
              <input type="checkbox" checked={r.reshoot} onChange={(e) => change(i, (c) => (c.reshoot = e.target.checked))} /> 그림도 다시 찍기
            </label>
            <span>→</span>
            <select value={r.until} onChange={(e) => change(i, (c) => (c.until = e.target.value as AutoRule["until"]))}>
              <option value="review">검토 대기까지</option>
              <option value="publish">수락하고 컨플루언스 게시까지</option>
            </select>
          </div>
          {r.scope === "features" && (
            <div className="auto-features">
              {r.features.map((k) => (
                <span key={k} className="chip">
                  {label(k)}
                  <button className="link" onClick={() => change(i, (c) => (c.features = c.features.filter((x) => x !== k)))}>✕</button>
                </span>
              ))}
              <select value="" onChange={(e) => e.target.value && change(i, (c) => c.features.push(e.target.value))}>
                <option value="">+ 메뉴 고르기</option>
                {features.filter((f) => !r.features.includes(f.key)).map((f) => (
                  <option key={f.key} value={f.key}>{f.label}</option>
                ))}
              </select>
            </div>
          )}
          <div className="auto-foot muted small">
            {r.until === "publish" && <span className="warn-text">검토 없이 제안을 받아들이고 게시합니다. </span>}
            다음 실행: {r.enabled ? when(r.next_run) : "꺼짐"} · 마지막: {when(r.last_run)}
            <span className="spacer" />
            <button className="btn small" disabled={!r.id || dirty || auto.running} title={!r.id || dirty ? "먼저 저장하세요" : ""} onClick={() => onCommand("auto", r.id)}>
              지금 실행
            </button>
            <button
              className="btn small"
              onClick={() => {
                const next = rules.filter((_, k) => k !== i);
                setRules(next);
                if (r.id) void save(next);
              }}
            >
              삭제
            </button>
          </div>
        </div>
      ))}

      {auto.runs.length > 0 && (
        <>
          <h3 className="auto-runs-title">실행 기록</h3>
          <table className="auto-runs">
            <thead>
              <tr>
                <th>시각</th>
                <th>규칙</th>
                <th>결과</th>
              </tr>
            </thead>
            <tbody>
              {auto.runs.slice(0, 15).map((run) => {
                const st = STATUS[run.status] ?? STATUS.failed;
                return (
                  <tr key={`${run.rule}-${run.started}`} className={run.status}>
                    <td>{when(run.started)}</td>
                    <td>{run.name}</td>
                    <td>
                      <details>
                        <summary>
                          {st.mark} {st.label}
                          {run.features.length > 0 && ` · 기능 ${run.features.length}개 (${run.features.map(label).join(", ")})`}
                        </summary>
                        <ul>
                          {run.lines.map((l, k) => (
                            <li key={k}>{l}</li>
                          ))}
                        </ul>
                      </details>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </>
      )}
    </section>
  );
}
