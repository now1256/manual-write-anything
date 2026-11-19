import { useEffect, useState } from "react";
import DOMPurify from "dompurify";
import { marked } from "marked";
import ModelPanel from "./ModelPanel";
import { fetchReferences, linkReference, refFileUrl, refPageUrl, uploadReference, type RefSummary } from "../api";

const isMarkdown = (path: string) => /\.(md|markdown)$/i.test(path);

interface Props {
  featureKey: string;
  version: number;
  onCommand: (name: string, arg?: string) => void;
  onNext: () => void;
  /** The paired old manual changed (linked, uploaded or unlinked). */
  onLinked: () => void;
}

/** Step ①: the old manual (PDF) this feature's manual starts from. */
export default function SourceStep({ featureKey, version, onCommand, onNext, onLinked }: Props) {
  const [all, setAll] = useState<RefSummary[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [reload, setReload] = useState(0);

  useEffect(() => {
    fetchReferences()
      .then((l) => setAll(l.items))
      .catch(() => setAll([]));
  }, [featureKey, version, reload]);

  if (all === null) return <p className="muted">불러오는 중…</p>;
  const doc = all.find((d) => d.feature === featureKey && !d.ignored);

  const link = async (id: string) => {
    try {
      setErr("");
      await linkReference(featureKey, id);
      setReload((n) => n + 1);
      onLinked();
    } catch (x) {
      setErr(String((x as Error).message ?? x));
    }
  };
  const upload = async (f: File) => {
    try {
      setErr("");
      setBusy(true);
      await uploadReference(featureKey, f);
      setReload((n) => n + 1);
      onLinked();
    } catch (x) {
      setErr(String((x as Error).message ?? x));
    } finally {
      setBusy(false);
    }
  };

  const picker = (
    <div className="step-actions">
      <label className="btn">
        {busy ? "올리는 중…" : "파일 올리기 (PDF · MD)"}
        <input
          type="file"
          accept=".pdf,.md,.markdown,application/pdf,text/markdown"
          hidden
          disabled={busy}
          onChange={(e) => {
            const f = e.target.files?.[0];
            e.target.value = "";
            if (f) void upload(f);
          }}
        />
      </label>
      {all.length > 0 && (
        <select value="" onChange={(e) => e.target.value && void link(e.target.value)}>
          <option value="">가져온 PDF에서 고르기…</option>
          {all
            .filter((d) => d.id !== doc?.id)
            .map((d) => (
              <option key={d.id} value={d.id}>
                {d.title}
                {d.tag ? ` (${d.tag})` : ""} · {isMarkdown(d.source) ? "MD" : `${d.pages}쪽`}{d.feature && d.feature !== featureKey ? " · 다른 기능에 연결됨" : ""}
              </option>
            ))}
        </select>
      )}
    </div>
  );

  if (!doc) {
    return (
      <div className="step">
        <h3>① 기존 매뉴얼 연결</h3>
        <p>이 기능의 기존 매뉴얼(PDF 또는 MD)을 연결하면, 그 글과 구조를 그대로 옮기고 그림은 지금 화면으로 바꿔 새 매뉴얼을 만듭니다.</p>
        {picker}
        {err && <p className="flash err">{err}</p>}
        <p className="muted small">기존 매뉴얼이 없는 기능은 이 단계를 건너뛰고 ③에서 코드로 새로 씁니다.</p>
      <details className="step-change models-wrap">
        <summary>모델 구성 — 단계마다 어떤 모델이 무엇을 하는지 (프로젝트 전체)</summary>
        <ModelPanel />
      </details>
      </div>
    );
  }

  const described = doc.described === doc.images;
  const md = isMarkdown(doc.source);
  return (
    <div className="step">
      <div className="step-head">
        <div>
          <h3>① 기존 매뉴얼</h3>
          <b>{doc.title}{doc.tag ? ` (${doc.tag})` : ""}</b>
          <span className="muted">
            {md ? " · 마크다운 (그림 없음)" : ` · ${doc.pages}쪽 · 그림 ${doc.images}장 · 장면 설명 ${doc.described}/${doc.images}`}
          </span>
          <div className="muted small mono">{doc.source}</div>
        </div>
        <span className="spacer" />
        {doc.images > 0 && (
          <button className={`btn ${described ? "" : "primary"}`} onClick={() => onCommand("describe", doc.id)}>
            {doc.described ? "장면 설명 다시 만들기" : "장면 설명 만들기"}
          </button>
        )}
        <button className="btn primary" disabled={!described} onClick={onNext} title={described ? "" : "장면 설명을 먼저 만드세요"}>
          ② 그림 고르기로 →
        </button>
      </div>
      <p className="muted small">
        장면 설명: 그림마다 어떤 화면·상태인지 모델이 읽어 둡니다. 이것이 있어야 ②에서 같은 장면을 찍을 수 있습니다 (기능마다 한 번, 몇 센트).
      </p>
      <details className="step-change">
        <summary>다른 PDF로 바꾸기 · 연결 해제</summary>
        {picker}
        <div className="step-actions">
          <button className="btn" onClick={() => void link("")}>
            연결 해제
          </button>
          <span className="muted small">
            이 기능을 기존 매뉴얼 없이 다룹니다. ③에서 다른 매뉴얼을 본보기로 처음부터 쓰고 그림은 지금 화면으로 찍습니다. 가져온 PDF는 남아 있어 다시 연결할 수 있습니다.
          </span>
        </div>
      </details>
      {err && <p className="flash err">{err}</p>}
      <details className="step-change models-wrap">
        <summary>모델 구성 — 단계마다 어떤 모델이 무엇을 하는지 (프로젝트 전체)</summary>
        <ModelPanel />
      </details>
      {md && <MarkdownPreview id={doc.id} />}
      <div className="pages">
        {Array.from({ length: doc.pages }, (_, i) => i + 1).map((n) => (
          <a key={n} href={refPageUrl(doc.id, n)} target="_blank" rel="noreferrer" className="page-thumb" title={`${n}쪽 크게 보기`}>
            <img src={refPageUrl(doc.id, n)} alt={`${n}쪽`} loading="lazy" />
            <span>{n}</span>
          </a>
        ))}
      </div>
    </div>
  );
}

/** The Markdown manual as it reads (stored as the document's text). */
function MarkdownPreview({ id }: { id: string }) {
  const [html, setHtml] = useState("");
  useEffect(() => {
    fetch(refFileUrl(id, "text.txt"))
      .then((r) => r.text())
      .then((t) => setHtml(DOMPurify.sanitize(marked.parse(t, { async: false }) as string)))
      .catch(() => setHtml(""));
  }, [id]);
  return <div className="md-preview doc-body" dangerouslySetInnerHTML={{ __html: html }} />;
}
