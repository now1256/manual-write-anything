import { useEffect, useRef, useState } from "react";
import DOMPurify from "dompurify";
import { useEditorState, type Editor } from "@tiptap/react";
import { DOMSerializer, type Node as PMNode } from "@tiptap/pm/model";
import { assist, refPageUrl, type AssistEdit, type AssistReply, type DocChange, type ManualFact, type ProvEntry, type Provenance } from "../api";
import { bidTexts, withBids } from "./blockIds";

export type PanelTab = "요청" | "기록";

export const KIND: Record<string, string> = {
  pdf: "기존 매뉴얼(PDF)",
  md: "기존 매뉴얼(MD)",
  draft: "코드로 새로 씀",
  proposal: "코드 기준 제안 수락",
  request: "도우미 요청",
  edit: "사람이 고침",
  restore: "되돌림",
  rewrite: "코드로 다시 씀",
  removed: "지움",
};

const NODE_KO: Record<string, string> = {
  paragraph: "문단",
  heading: "제목",
  table: "표",
  bulletList: "목록",
  orderedList: "번호 목록",
  taskList: "작업 목록",
  panel: "패널",
  figure: "그림",
  blockquote: "인용",
  codeBlock: "코드",
  details: "펼치기",
};

const ALLOWED_ATTR = ["data-bid", "data-panel", "data-status", "data-ref", "data-asset", "data-shot", "data-width"];

/** "표 '용어 의미 주문…'" for a block. */
export function labelOf(node: PMNode): string {
  const text = node.textBetween(0, node.content.size, " ") || (node.attrs.caption as string) || "";
  return `${NODE_KO[node.type.name] ?? "블록"} '${text.slice(0, 24)}'`;
}

/** Top-level blocks with these ids, in document order, with their positions. */
function blocksOf(editor: Editor, bids: string[]) {
  const out: { node: PMNode; from: number; to: number }[] = [];
  editor.state.doc.forEach((node, offset) => {
    if (bids.includes(node.attrs.bid)) out.push({ node, from: offset, to: offset + node.nodeSize });
  });
  return out;
}

/** HTML of top-level nodes, as the editor would save them. */
function htmlOf(editor: Editor, nodes: PMNode[]): string {
  const ser = DOMSerializer.fromSchema(editor.state.schema);
  const box = document.createElement("div");
  for (const n of nodes) box.appendChild(ser.serializeNode(n));
  return box.innerHTML;
}

/** Ids of the top-level blocks under the selection (or the cursor). */
export function focusOf(editor: Editor): { bids: string[]; label: string } | null {
  const { doc, selection } = editor.state;
  const nodes: PMNode[] = [];
  doc.forEach((node, offset) => {
    if (offset + node.nodeSize > selection.from && offset < Math.max(selection.to, selection.from + 1)) nodes.push(node);
  });
  if (!nodes.length) return null;
  return {
    bids: nodes.map((n) => n.attrs.bid as string | null).filter((b): b is string => !!b),
    label: labelOf(nodes[0]) + (nodes.length > 1 ? ` 외 ${nodes.length - 1}개` : ""),
  };
}

export function FactChips({ ids, facts }: { ids: string[]; facts: Map<string, ManualFact> }) {
  if (!ids.length) return null;
  return (
    <div className="fact-list">
      {ids.map((id) => {
        const f = facts.get(id);
        return (
          <details key={id} className="fact">
            <summary>
              <span className="mono">{id}</span> {f?.statement ?? "(지금 코드 사실에 없음 — 코드를 다시 읽었을 수 있음)"}
            </summary>
            {f?.source && (
              <div className="small">
                <div className="mono muted">
                  {f.source.repo} · {f.source.file}:{f.source.line_start}
                  {f.source.line_end > f.source.line_start ? `–${f.source.line_end}` : ""}
                </div>
                <pre className="fact-quote">{f.source.quote}</pre>
              </div>
            )}
          </details>
        );
      })}
    </div>
  );
}

export function EntryItem({ e, facts, refId }: { e: ProvEntry; facts: Map<string, ManualFact>; refId: string | null }) {
  return (
    <li className={`entry ${e.kind}`}>
      <div className="entry-head">
        <b>{KIND[e.kind] ?? e.kind}</b>
        <span className="muted small">
          {e.version ? `버전 ${e.version}` : "처음"}
          {e.at ? ` · ${new Date(e.at * 1000).toLocaleString("ko-KR", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}` : ""}
        </span>
      </div>
      <div>{e.summary}</div>
      {e.request && <div className="entry-req">요청: “{e.request}”</div>}
      {e.page && refId && (
        <a href={refPageUrl(refId, e.page)} target="_blank" rel="noreferrer" className="entry-page">
          <img src={refPageUrl(refId, e.page)} alt={`${e.page}쪽`} loading="lazy" />
        </a>
      )}
      <FactChips ids={e.facts ?? []} facts={facts} />
    </li>
  );
}

/** One proposed edit as shown before applying: where, and what is there now. */
interface Planned {
  edit: AssistEdit;
  where: string;
  before: string;
  /** Block to jump to. */
  at: string;
}

interface Turn {
  id: number;
  who: "user" | "assistant";
  text: string;
  reply?: AssistReply;
  planned?: Planned[];
  status?: "applied" | "dismissed";
  request?: string;
  focus?: string;
}

let turnId = 1;

interface Props {
  editor: Editor;
  featureKey: string;
  prov: Provenance;
  facts: Map<string, ManualFact>;
  pending: DocChange[];
  onChange: (c: DocChange) => void;
  tab: PanelTab;
  setTab: (t: PanelTab) => void;
  /** A request sent from the right-click menu; `n` changes per click. */
  ask: { text?: string; n: number } | null;
  renderDiff: (older: string, newer: string) => React.ReactNode;
  onGotoBid: (bid: string) => void;
  /** Shows a result large in the main area. */
  onPreview: (older: string, newer: string) => void;
}

/** Side panel of the editor: ask for changes, and what changed per version. */
export default function DocSidePanel(props: Props) {
  const { tab, setTab } = props;
  return (
    <aside className="side">
      <div className="side-tabs" role="tablist">
        {(["요청", "기록"] as PanelTab[]).map((t) => (
          <button key={t} role="tab" aria-selected={t === tab} className={t === tab ? "on" : ""} onClick={() => setTab(t)}>
            {t}
          </button>
        ))}
      </div>
      <div className="side-body">
        <div hidden={tab !== "요청"}>
          <RequestTab {...props} />
        </div>
        {tab === "기록" && <LogTab {...props} />}
      </div>
    </aside>
  );
}

function RequestTab({ editor, featureKey, facts, onChange, ask, renderDiff, onGotoBid, onPreview }: Props) {
  const [turns, setTurns] = useState<Turn[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);
  const end = useRef<HTMLDivElement>(null);
  const looking = useEditorState({ editor, selector: ({ editor: e }) => focusOf(e)?.label ?? null });

  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" });
  }, [turns, busy]);

  /** Where each edit lands and what is there now (for the preview). */
  const plan = (edits: AssistEdit[]): Planned[] =>
    edits.map((edit) => {
      if (edit.replace.length) {
        const blocks = blocksOf(editor, edit.replace);
        return {
          edit,
          where: blocks.length ? labelOf(blocks[0].node) + (blocks.length > 1 ? ` 외 ${blocks.length - 1}개` : "") : "(찾지 못함)",
          before: htmlOf(editor, blocks.map((b) => b.node)),
          at: edit.replace[0],
        };
      }
      const after = blocksOf(editor, [edit.after])[0];
      return { edit, where: after ? `${labelOf(after.node)} 뒤` : "(찾지 못함)", before: "", at: edit.after };
    });

  const send = async (text: string) => {
    const req = text.trim();
    if (!req || busy) return;
    const focus = focusOf(editor);
    const history = turns.map((x) => ({ who: x.who, text: x.who === "user" ? x.text : x.reply?.answer ?? "" }));
    setTurns((all) => [...all, { id: turnId++, who: "user", text: req, focus: focus?.label }]);
    setDraft("");
    setErr("");
    setBusy(true);
    try {
      const reply = await assist(featureKey, { request: req, html: editor.getHTML(), focus: focus?.bids ?? [], history });
      setTurns((all) => [...all, { id: turnId++, who: "assistant", text: reply.answer, reply, planned: plan(reply.edits), request: req }]);
    } catch (x) {
      setErr(String((x as Error).message ?? x));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    if (!ask) return;
    input.current?.focus();
    if (ask.text) void send(ask.text);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ask]);

  const apply = (turn: Turn) => {
    const planned = turn.planned ?? [];
    // Positions now; applied bottom-up so earlier ranges stay valid.
    const steps = planned.map((p) => {
      const next = withBids(DOMPurify.sanitize(p.edit.html, { ADD_ATTR: ALLOWED_ATTR }));
      if (p.edit.replace.length) {
        const blocks = blocksOf(editor, p.edit.replace);
        if (blocks.length !== p.edit.replace.length) return null;
        return { p, next, from: blocks[0].from, to: blocks[blocks.length - 1].to };
      }
      const after = blocksOf(editor, [p.edit.after])[0];
      return after ? { p, next, from: after.to, to: after.to } : null;
    });
    if (steps.some((s) => !s)) return setErr("바꿀 곳이 그사이 바뀌어 적용하지 못했습니다. 다시 요청하세요.");
    const ok = steps as NonNullable<(typeof steps)[number]>[];
    const chain = editor.chain().focus();
    for (const s of [...ok].sort((a, b) => b.from - a.from)) {
      if (s.next.html.trim()) chain.insertContentAt({ from: s.from, to: s.to }, s.next.html);
      else chain.deleteRange({ from: s.from, to: s.to });
    }
    chain.run();
    const summary = (turn.reply?.answer.split("\n")[0] || turn.request || "").slice(0, 120);
    for (const s of ok) {
      const before = bidTexts(s.p.before);
      const after = bidTexts(s.next.html);
      onChange({
        bids: s.next.bids.filter((b) => !before.has(b) || before.get(b) !== after.get(b)),
        kind: "request",
        summary,
        request: turn.request ?? null,
        facts: turn.reply?.facts ?? [],
        inherit: Array.from(before.keys()).filter((b) => !after.has(b)),
      });
    }
    setTurns((all) => all.map((x) => (x.id === turn.id ? { ...x, status: "applied" } : x)));
  };

  const planned = (t: Turn) => t.planned ?? [];

  return (
    <div className="req">
      <div className="req-log">
        {turns.length === 0 && (
          <div className="req-help">
            <p>
              <b>무엇을 고칠지 말로 요청하세요.</b> 도우미가 문서 전체를 보고 고칠 곳을 찾아, 바뀔 내용을 먼저 보여 줍니다. 적용을 눌러야 문서가 바뀝니다.
            </p>
            <ul>
              <li>이 표가 잘렸어. 아래 표랑 하나로 합쳐 줘</li>
              <li>이 설명이 지금 코드와 맞아?</li>
              <li>용어 설명 표에 NAS 행 추가해 줘</li>
              <li>문서 전체를 프로젝트 양식 순서로 맞춰 줘</li>
            </ul>
            <p className="muted small">편집기에서 지금 보고 있는 곳을 누르면 거기서부터 찾습니다. 정확히 선택하지 않아도 됩니다.</p>
          </div>
        )}
        {turns.map((t) =>
          t.who === "user" ? (
            <div key={t.id} className="turn user">
              {t.focus && <span className="turn-target">보던 곳: {t.focus}</span>}
              {t.text}
            </div>
          ) : (
            <div key={t.id} className={`turn assistant ${t.status ?? ""}`}>
              <p className="turn-answer">{t.text}</p>
              <FactChips ids={t.reply?.facts ?? []} facts={facts} />
              {planned(t).map((p, i) => (
                <div key={i} className="turn-edit">
                  <div className="turn-where">
                    <b>{p.edit.replace.length ? (p.edit.html.trim() ? "바꿀 곳" : "지울 곳") : "추가할 곳"}</b> {p.where}{" "}
                    <button className="link" onClick={() => onGotoBid(p.at)}>
                      위치 보기
                    </button>
                  </div>
                  {renderDiff(p.before, p.edit.html)}
                </div>
              ))}
              {planned(t).length > 0 &&
                (t.status === "applied" ? (
                  <p className="ok small">✓ 적용했습니다 (저장하면 버전에 남습니다)</p>
                ) : t.status === "dismissed" ? (
                  <p className="muted small">취소했습니다</p>
                ) : (
                  <div className="turn-actions">
                    <button className="btn small primary" onClick={() => apply(t)}>
                      적용{planned(t).length > 1 ? ` (${planned(t).length}곳)` : ""}
                    </button>
                    <button
                      className="btn small"
                      onClick={() => onPreview(planned(t).map((p) => p.before).join(""), planned(t).map((p) => p.edit.html).join(""))}
                    >
                      크게 보기
                    </button>
                    <button className="btn small" onClick={() => t.request && void send(t.request)} disabled={busy}>
                      다시
                    </button>
                    <button className="btn small" onClick={() => setTurns((all) => all.map((x) => (x.id === t.id ? { ...x, status: "dismissed" } : x)))}>
                      취소
                    </button>
                  </div>
                ))}
            </div>
          ),
        )}
        {busy && (
          <div className="turn assistant busy">
            <span className="spinner" /> 도우미가 문서를 보고 답하는 중…
          </div>
        )}
        {err && <p className="flash err">{err}</p>}
        <div ref={end} />
      </div>
      <form
        className="req-input"
        onSubmit={(e) => {
          e.preventDefault();
          void send(draft);
        }}
      >
        <div className="req-scope muted">보고 있는 곳: {looking ?? "(편집기에서 누르면 거기서부터 찾습니다)"}</div>
        <textarea
          ref={input}
          rows={3}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send(draft);
            }
          }}
          placeholder="요청을 입력하세요 (Enter 보내기, Shift+Enter 줄바꿈)"
        />
        <button className="btn primary" type="submit" disabled={busy || !draft.trim()}>
          보내기
        </button>
      </form>
    </div>
  );
}

function LogTab({ prov, pending, onGotoBid }: Props) {
  return (
    <div className="log">
      {pending.length > 0 && (
        <section>
          <h4>
            저장 전 <span className="badge warn">{pending.length}</span>
          </h4>
          <ul>
            {pending.map((c, i) => (
              <li key={i}>
                <button className="link" onClick={() => c.bids[0] && onGotoBid(c.bids[0])}>
                  {KIND[c.kind] ?? c.kind}
                </button>{" "}
                — {c.summary}
              </li>
            ))}
          </ul>
        </section>
      )}
      {[...prov.log].reverse().map((v) => (
        <section key={v.version}>
          <h4>
            {v.version ? `버전 ${v.version}` : "처음 만듦"} <span className="muted small">{new Date(v.at * 1000).toLocaleString("ko-KR")}</span>
          </h4>
          {v.items.length === 0 ? (
            <p className="muted small">내용 변경 없음</p>
          ) : (
            <ul>
              {v.items.map((it, i) => (
                <li key={i}>
                  <button className="link" onClick={() => onGotoBid(it.bid)} disabled={it.kind === "removed"}>
                    {KIND[it.kind] ?? it.kind}
                  </button>{" "}
                  — {it.summary}
                </li>
              ))}
            </ul>
          )}
        </section>
      ))}
      {prov.log.length === 0 && <p className="muted">아직 기록이 없습니다. 저장하면 버전마다 무엇을 왜 바꿨는지 쌓입니다.</p>}
    </div>
  );
}
