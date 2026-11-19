import { useEffect, useState } from "react";
import { fetchManual, type ManualRun } from "../api";

interface Props {
  featureKey: string;
  /** Bumped when the code was read again. */
  version: number;
  onCommand: (name: string, arg?: string) => void;
}

const KIND: Record<string, string> = {
  label: "화면 글자",
  rule: "입력 규칙",
  behavior: "동작",
  message: "메시지",
  permission: "권한",
};

/** What "코드 읽기" found: the facts manuals are written from, with where each comes from. */
export default function FactsView({ featureKey, version, onCommand }: Props) {
  const [run, setRun] = useState<ManualRun | null | undefined>(undefined);
  const [q, setQ] = useState("");
  useEffect(() => {
    setRun(undefined);
    fetchManual(featureKey)
      .then(setRun)
      .catch(() => setRun(null));
  }, [featureKey, version]);

  if (run === undefined) return <p className="muted">불러오는 중…</p>;
  if (!run)
    return (
      <div className="empty">
        <p>아직 이 기능의 코드를 읽지 않았습니다.</p>
        <button className="btn primary" onClick={() => onCommand("gen", featureKey)}>코드 읽기</button>
      </div>
    );
  const words = q.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const shown = run.facts.filter((f) => {
    const hay = `${f.id} ${f.statement} ${f.source?.file ?? ""}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
  return (
    <div className="facts-view">
      <div className="facts-head">
        <b>코드 사실 {run.facts.length}개</b>
        <span className="muted small">
          {new Date(run.generated_at * 1000).toLocaleString("ko-KR")} 기준
          {run.dropped_facts ? ` · 근거 줄이 맞지 않아 버림 ${run.dropped_facts}` : ""}
        </span>
        <span className="spacer" />
        <input className="facts-search" placeholder="찾기 (예: 삭제, F012)" value={q} onChange={(e) => setQ(e.target.value)} />
        <button className="btn" onClick={() => onCommand("gen", featureKey)}>코드 다시 읽기</button>
      </div>
      <p className="muted small">③의 매뉴얼 쓰기와 ④의 근거 확인·도우미가 이 사실을 근거로 씁니다. 사실마다 코드의 어느 줄에서 나왔는지 펼쳐 볼 수 있습니다.</p>
      <ul className="facts-list">
        {shown.map((f) => (
          <li key={f.id}>
            <details>
              <summary>
                <span className="mono fact-id">{f.id}</span>
                <span className="badge">{KIND[f.kind] ?? f.kind}</span> {f.statement}
              </summary>
              {f.source ? (
                <div className="small">
                  <div className="mono muted">
                    {f.source.repo} · {f.source.file}:{f.source.line_start}
                    {f.source.line_end > f.source.line_start ? `–${f.source.line_end}` : ""}
                  </div>
                  <pre className="fact-quote">{f.source.quote}</pre>
                </div>
              ) : (
                <p className="muted small">제품 지도에서 나온 사실 ({f.basis})</p>
              )}
            </details>
          </li>
        ))}
      </ul>
    </div>
  );
}
