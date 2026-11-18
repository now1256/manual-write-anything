import type { Status } from "../api";

const WINDOW: Record<string, string> = { seven_day: "7일", five_hour: "5시간" };

interface Props {
  status: Status | null;
  models: string[];
  busy: string | null;
  connected: boolean;
  onModel: (model: string) => void;
}

export default function StatusBar({ status, models, busy, connected, onModel }: Props) {
  const q = status?.quota;
  // status.model is "backend·model"
  const [backend, current] = (status?.model ?? "…").split("·");
  return (
    <footer className="statusbar">
      {models.length > 1 ? (
        <label className="chip mono model-pick" title="대화에 쓸 모델">
          {backend}·
          <select value={current} disabled={!!busy} onChange={(e) => onModel(e.target.value)}>
            {models.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </label>
      ) : (
        <span className="chip mono">{status?.model ?? "…"}</span>
      )}
      {status && (
        <span title="구독 사용량을 API 정가로 환산한 값입니다. 따로 청구되지 않습니다">
          사용량(정가 환산) ${status.spent_usd.toFixed(3)} / ${status.budget_usd.toFixed(2)}
        </span>
      )}
      {q?.utilization != null && (
        <span>
          {WINDOW[q.window] ?? q.window} 한도 {(q.utilization * 100).toFixed(0)}%
        </span>
      )}
      {busy && (
        <span className="busy">
          <span className="spinner" aria-hidden /> {busy} 중…
        </span>
      )}
      <span className="spacer" />
      <span className={connected ? "conn up" : "conn down"}>{connected ? "● 서버 연결됨" : "○ 연결 끊김 — 다시 연결 중"}</span>
    </footer>
  );
}
