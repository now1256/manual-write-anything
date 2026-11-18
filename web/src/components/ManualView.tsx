import { useEffect, useMemo, useState } from "react";
import DOMPurify from "dompurify";
import { marked } from "marked";
import { fetchManual, shotUrl, type BlockCheck, type ManualBlock, type ManualFact, type ManualRun } from "../api";

interface Props {
  featureKey: string;
  /** Bumped when a manual was regenerated. */
  version: number;
  onCommand: (name: string) => void;
}

const VERDICT: Record<string, { mark: string; cls: string; label: string }> = {
  supported: { mark: "✓", cls: "ok", label: "근거 확인" },
  partial: { mark: "◐", cls: "warn", label: "일부 근거 없음" },
  unsupported: { mark: "✗", cls: "err", label: "근거 없음" },
  skipped: { mark: "", cls: "", label: "" },
};

function html(md: string): string {
  return DOMPurify.sanitize(marked.parse(md, { async: false }) as string);
}

export default function ManualView({ featureKey, version, onCommand }: Props) {
  const [run, setRun] = useState<ManualRun | null | undefined>(undefined);
  const [showChecks, setShowChecks] = useState(true);
  const [open, setOpen] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setRun(undefined);
    fetchManual(featureKey)
      .then((r) => live && setRun(r))
      .catch(() => live && setRun(null));
    return () => {
      live = false;
    };
  }, [featureKey, version]);

  const facts = useMemo(() => new Map((run?.facts ?? []).map((f) => [f.id, f])), [run]);
  const checks = useMemo(() => new Map((run?.checks ?? []).map((c) => [c.id, c])), [run]);

  if (run === undefined) return <p className="muted">불러오는 중…</p>;
  if (run === null) {
    return (
      <div className="placeholder">
        아직 이 기능의 매뉴얼이 없습니다. <button className="btn" onClick={() => onCommand("gen")}>/gen 실행</button>
      </div>
    );
  }

  const count = (v: string) => run.checks.filter((c) => c.verdict === v).length;
  const todos = run.blocks.filter((b) => b.kind === "todo").length;
  let shotNo = 0;

  return (
    <div className="manual">
      <div className="manual-bar">
        <span className="badge ok">근거 확인 {count("supported")}</span>
        {count("partial") > 0 && <span className="badge warn">일부 {count("partial")}</span>}
        {count("unsupported") > 0 && <span className="badge err">근거 없음 {count("unsupported")}</span>}
        {todos > 0 && <span className="badge warn">확인 필요 {todos}</span>}
        <span className="muted small">
          사실 {run.facts.length} · ${run.cost_usd.toFixed(2)} · {new Date(run.generated_at * 1000).toLocaleString("ko-KR")}
        </span>
        <span className="spacer" />
        <label className="small muted">
          <input type="checkbox" checked={showChecks} onChange={(e) => setShowChecks(e.target.checked)} /> 검증 표시
        </label>
        <button className="btn ghost" onClick={() => onCommand("gen")}>다시 만들기</button>
      </div>

      <article className="doc">
        <h1>{run.title}</h1>
        {run.blocks.map((b) => {
          const no = b.kind === "image" ? ++shotNo : 0;
          return (
            <Block
              key={b.id}
              block={b}
              shotNo={no}
              check={checks.get(b.id)}
              facts={b.facts.map((id) => facts.get(id)).filter((f): f is ManualFact => !!f)}
              showChecks={showChecks}
              open={open === b.id}
              onToggle={() => setOpen(open === b.id ? null : b.id)}
            />
          );
        })}
      </article>

      {run.compare && <Compare run={run} />}
    </div>
  );
}

function Block(props: {
  block: ManualBlock;
  shotNo: number;
  check: BlockCheck | undefined;
  facts: ManualFact[];
  showChecks: boolean;
  open: boolean;
  onToggle: () => void;
}) {
  const { block: b, shotNo, check, facts, showChecks, open, onToggle } = props;
  const v = VERDICT[check?.verdict ?? "skipped"] ?? VERDICT.skipped;
  const issues = check?.issues ?? [];
  let body;
  if (b.kind === "image" && b.shot) {
    const caption = b.markdown.trim();
    body = (
      <figure>
        <figcaption>
          <b>[사진 {shotNo}]</b> {caption}
        </figcaption>
        <img src={shotUrl(`shots/${b.shot}.png`, null)} alt={caption || `사진 ${shotNo}`} />
      </figure>
    );
  } else if (b.kind === "todo") {
    body = <div className="todo" dangerouslySetInnerHTML={{ __html: html(b.markdown) }} />;
  } else if (b.kind === "callout") {
    body = <div className="callout" dangerouslySetInnerHTML={{ __html: html(b.markdown) }} />;
  } else {
    body = <div dangerouslySetInnerHTML={{ __html: html(b.markdown) }} />;
  }
  const checkable = v.mark !== "";
  return (
    <section className={`blk ${showChecks && checkable ? `v-${v.cls}` : ""}`}>
      {showChecks && checkable && (
        <button className={`blk-mark ${v.cls}`} title={`${v.label}${check?.reason ? `: ${check.reason}` : ""}`} onClick={onToggle}>
          {v.mark}
        </button>
      )}
      {body}
      {showChecks && open && (
        <div className="evidence">
          {check?.reason && <p className="small">{check.reason}</p>}
          {issues.map((i) => (
            <p key={i} className="small warn-text">
              {i}
            </p>
          ))}
          {facts.map((f) => (
            <div key={f.id} className="fact small">
              <b className="mono">{f.id}</b> {f.statement}
              {f.source && (
                <div className="mono muted">
                  {f.source.repo}:{f.source.file}:{f.source.line_start} — <code>{f.source.quote}</code>
                </div>
              )}
              {f.basis === "code" && f.kind === "behavior" && <span className="badge">코드 기준 · 실행 미확인</span>}
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

const STATUS: Record<string, string> = { same: "같음", only_existing: "기존에만", only_generated: "새로 추가", conflict: "충돌" };

function Compare({ run }: { run: ManualRun }) {
  const items = run.compare ?? [];
  const order = ["conflict", "only_existing", "only_generated", "same"];
  const sorted = [...items].sort((a, b) => order.indexOf(a.status) - order.indexOf(b.status));
  return (
    <section className="compare">
      <h3>기존 매뉴얼과 비교 · {run.compare_source}</h3>
      <table>
        <thead>
          <tr>
            <th>상태</th>
            <th>주제</th>
            <th>새 매뉴얼</th>
            <th>기존 매뉴얼</th>
            <th>코드 판정</th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((it, i) => (
            <tr key={i} className={`st-${it.status}`}>
              <td>
                <span className={`badge ${it.status === "conflict" ? "err" : it.status === "same" ? "ok" : "warn"}`}>{STATUS[it.status]}</span>
              </td>
              <td>{it.topic}</td>
              <td>{it.generated ?? "—"}</td>
              <td>{it.existing ?? "—"}</td>
              <td className="small">
                {it.code_supports === "generated" && <b className="ok-text">새 매뉴얼이 맞음 </b>}
                {it.code_supports === "existing" && <b className="err-text">기존이 맞음 </b>}
                {it.code_says ?? ""}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
