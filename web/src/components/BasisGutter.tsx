import { useEffect, useRef, useState } from "react";
import type { Editor } from "@tiptap/react";
import type { DocChange, Ground, Grounding, ManualFact, Provenance } from "../api";
import { EntryItem, FactChips, KIND, labelOf } from "./DocSidePanel";

interface Mark {
  bid: string;
  top: number;
  /** Verdict of the last check, or `stale` (text changed since) / `unchecked`. */
  verdict: string;
  label: string;
}

const VERDICT: Record<string, { label: string; mark: string }> = {
  supported: { label: "코드로 확인됨", mark: "✓" },
  partial: { label: "일부만 코드로 확인", mark: "◐" },
  contradicted: { label: "코드와 다름", mark: "!" },
  unsupported: { label: "코드에 근거 없음", mark: "?" },
  no_claim: { label: "확인할 내용 없음", mark: "·" },
  stale: { label: "글이 바뀌어 다시 확인 필요", mark: "↻" },
  unchecked: { label: "아직 근거 확인 안 함", mark: "?" },
  picture: { label: "그림 — ② 그림 고르기에서 정함", mark: "▣" },
};

const flat = (t: string) => t.replace(/\s+/g, "");

interface Props {
  editor: Editor;
  prov: Provenance;
  grounding: Grounding;
  pending: DocChange[];
  facts: Map<string, ManualFact>;
  refId: string | null;
  /** Opens one block's basis (from the right-click menu); `n` changes per click. */
  open: { bid: string; n: number } | null;
}

/**
 * A "?" beside every block of the manual; clicking it shows where the block came from (old
 * manual page, code facts, requests, edits). Lives next to the editor, not inside it, so it never
 * gets in the way of typing.
 */
export default function BasisGutter({ editor, prov, grounding, pending, facts, refId, open }: Props) {
  const box = useRef<HTMLDivElement>(null);
  const [marks, setMarks] = useState<Mark[]>([]);
  const [shown, setShown] = useState<string | null>(null);

  useEffect(() => {
    let frame = 0;
    const measure = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const host = box.current?.getBoundingClientRect();
        if (!host || !editor.isInitialized) return;
        const out: Mark[] = [];
        editor.state.doc.forEach((node, offset) => {
          const bid = node.attrs.bid as string | null;
          const dom = editor.view.nodeDOM(offset) as HTMLElement | null;
          if (!bid || !dom?.getBoundingClientRect) return;
          const text = node.textBetween(0, node.content.size, " ");
          const g: Ground | undefined = grounding.blocks[bid];
          const verdict = node.type.name === "figure" ? "picture" : !g ? "unchecked" : flat(g.text) !== flat(text) ? "stale" : g.verdict;
          out.push({ bid, top: dom.getBoundingClientRect().top - host.top, verdict, label: labelOf(node) });
        });
        setMarks(out);
      });
    };
    measure();
    editor.on("update", measure);
    const ro = new ResizeObserver(measure);
    if (editor.isInitialized) ro.observe(editor.view.dom);
    window.addEventListener("resize", measure);
    return () => {
      cancelAnimationFrame(frame);
      editor.off("update", measure);
      ro.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, [editor, grounding]);

  useEffect(() => {
    if (open) setShown(open.bid);
  }, [open]);

  useEffect(() => {
    if (!shown) return;
    const off = (e: MouseEvent) => !(e.target as HTMLElement).closest(".basis-pop, .basis-mark") && setShown(null);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setShown(null);
    document.addEventListener("mousedown", off);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", off);
      document.removeEventListener("keydown", esc);
    };
  }, [shown]);

  const mark = marks.find((m) => m.bid === shown);
  const history = (shown && prov.blocks[shown]) || [];
  const ground = shown ? grounding.blocks[shown] : undefined;
  const unsaved = pending.filter((c) => shown && c.bids.includes(shown));

  return (
    <div className="basis-gutter" ref={box} contentEditable={false}>
      {marks.map((m) => (
        <button
          key={m.bid}
          className={`basis-mark v-${m.verdict} ${shown === m.bid ? "on" : ""}`}
          style={{ top: m.top }}
          title={`근거: ${VERDICT[m.verdict]?.label ?? ""}`}
          onClick={() => setShown(shown === m.bid ? null : m.bid)}
        >
          {VERDICT[m.verdict]?.mark ?? "?"}
        </button>
      ))}
      {mark && (
        <div className="basis-pop" style={{ top: mark.top }}>
          <div className="basis-pop-head">
            <b>근거</b> <span className="muted small">{mark.label}</span>
            <span className="spacer" />
            <button className="notice-x" onClick={() => setShown(null)} title="닫기">
              ×
            </button>
          </div>
          <section className={`why v-${mark.verdict}`}>
            <div className="why-head">
              <b>왜 이렇게 썼나</b>
              <span className="why-verdict">{VERDICT[mark.verdict]?.label}</span>
            </div>
            {mark.verdict === "picture" ? (
              <p className="muted small">그림은 글이 아니라서 코드와 대조하지 않습니다. 어떤 그림을 쓸지는 ② 그림 고르기에서 정하고, 그림 위 이름표(지금 화면·예전 화면 등)로 확인합니다.</p>
            ) : ground ? (
              <>
                <p>{ground.why}</p>
                <FactChips ids={ground.facts} facts={facts} />
                {mark.verdict === "stale" && <p className="muted small">위 판단은 고치기 전 글에 대한 것입니다. 저장 후 "근거 확인"을 다시 누르세요.</p>}
              </>
            ) : (
              <p className="muted small">위쪽 "근거 확인"을 누르면 이 문단을 코드 사실과 대조해, 무엇을 근거로 썼는지와 맞는지 여부를 여기에 보여 줍니다.</p>
            )}
          </section>
          <div className="hist-head">어디서 왔나</div>
          {history.length === 0 && unsaved.length === 0 && (
            <p className="muted small">기록이 없습니다. 이번에 새로 쓴 부분이면 저장할 때 "사람이 새로 씀"으로 남습니다.</p>
          )}
          <ol className="entries">
            {history.map((e, i) => (
              <EntryItem key={i} e={e} facts={facts} refId={refId} />
            ))}
            {unsaved.map((c, i) => (
              <li key={`u${i}`} className="entry unsaved">
                <div className="entry-head">
                  <b>{KIND[c.kind] ?? c.kind}</b> <span className="badge warn">저장 전</span>
                </div>
                <div>{c.summary}</div>
                {c.request && <div className="entry-req">요청: “{c.request}”</div>}
                <FactChips ids={c.facts} facts={facts} />
              </li>
            ))}
          </ol>
        </div>
      )}
    </div>
  );
}
