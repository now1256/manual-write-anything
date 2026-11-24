import { createContext, useContext, useEffect, useMemo, useRef, useState } from "react";
import DOMPurify from "dompurify";
import {
  EditorContent,
  Mark,
  Node,
  NodeViewWrapper,
  ReactNodeViewRenderer,
  mergeAttributes,
  useEditor,
  useEditorState,
  type Editor,
  type ReactNodeViewProps,
} from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { Table, TableCell, TableHeader, TableRow } from "@tiptap/extension-table";
import { Color, TextStyle } from "@tiptap/extension-text-style";
import TextAlign from "@tiptap/extension-text-align";
import { TaskItem, TaskList } from "@tiptap/extension-list";
import { Placeholder } from "@tiptap/extensions";
import { Details, DetailsContent, DetailsSummary } from "@tiptap/extension-details";
import { pictureResolver, type Pic } from "../pictures";
import { useContextMenu } from "./EditorAssist";
import { BlockIds, OutlineLook, withBids } from "./blockIds";
import DocSidePanel, { focusOf, type PanelTab } from "./DocSidePanel";
import BasisGutter from "./BasisGutter";
import { joinTables, pairBelowCursor, splitTables, type SplitPair } from "./tableMerge";
import ConfluencePublish from "./ConfluencePublish";
import { PanelBubble, SelectionBubble, SlashMenu, TableBubble, Toolbar } from "./EditorChrome";
import {
  fetchDoc,
  fetchDocVersion,
  fetchDocVersions,
  fetchProposals,
  fetchProvenance,
  fetchGrounding,
  fetchConfluence,
  fetchManual,
  resolveProposal,
  setSceneUpload,
  restoreDoc,
  fetchReference,
  fetchReferences,
  fetchSceneShots,
  saveDoc,
  uploadDocAsset,
  type ManualDoc,
  type Proposal,
  type DocChange,
  type ManualFact,
  type Provenance,
  type Grounding,
  type SceneShots,
} from "../api";

/** Where a picture comes from, resolved at render time (D-032). */

interface Ctx {
  resolve: (ref: string | null, asset: string | null, shot: string | null) => Pic;
  upload: (f: File) => Promise<string>;
  uploadScene: (ref: string, f: File) => Promise<void>;
  gotoReference: (ref: string) => void;
  /** Shoots a picture slot (its block id) on the product. */
  shootSlot: (bid: string) => void;
}

const DocCtx = createContext<Ctx | null>(null);

/** Confluence-style info/note/warning/success box. */
const Panel = Node.create({
  name: "panel",
  group: "block",
  content: "block+",
  defining: true,
  addAttributes() {
    return {
      type: {
        default: "info",
        parseHTML: (el) => el.getAttribute("data-panel") ?? "info",
        renderHTML: (a) => ({ "data-panel": a.type }),
      },
    };
  },
  parseHTML() {
    return [{ tag: "div[data-panel]" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["div", mergeAttributes(HTMLAttributes, { class: "panel" }), 0];
  },
});

/** Small coloured label (활성, 비활성 …). */
const Status = Mark.create({
  name: "status",
  // Typing after a label does not extend it.
  inclusive: false,
  addAttributes() {
    return {
      color: {
        default: "grey",
        parseHTML: (el) => el.getAttribute("data-status") ?? "grey",
        renderHTML: (a) => ({ "data-status": a.color }),
      },
    };
  },
  parseHTML() {
    return [{ tag: "span[data-status]" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["span", mergeAttributes(HTMLAttributes, { class: "status" }), 0];
  },
});

const dataAttr = (name: string) => ({
  default: null,
  parseHTML: (el: HTMLElement) => el.getAttribute(`data-${name}`),
  renderHTML: (a: Record<string, string | null>) => (a[name] ? { [`data-${name}`]: a[name] } : {}),
});

/**
 * Picture with caption: `ref` = old manual picture, `asset` = uploaded, `shot` = capture.
 * `want` makes it a picture slot: what the picture must show, shot automatically (`fail`: why not).
 */
const Figure = Node.create({
  name: "figure",
  group: "block",
  atom: true,
  draggable: true,
  addAttributes() {
    return {
      ref: dataAttr("ref"),
      asset: dataAttr("asset"),
      shot: dataAttr("shot"),
      want: dataAttr("want"),
      fail: dataAttr("fail"),
      /** Pictures side by side share a row (as the old manual laid them out). */
      row: dataAttr("row"),
      /** Width in % of the page; null = natural size. */
      width: {
        default: null,
        parseHTML: (el) => {
          const w = Number(el.getAttribute("data-width"));
          return w > 0 ? w : null;
        },
        renderHTML: (a) => (a.width ? { "data-width": String(a.width) } : {}),
      },
      caption: {
        default: "",
        parseHTML: (el) => el.querySelector("figcaption")?.textContent ?? "",
        renderHTML: () => ({}),
      },
    };
  },
  parseHTML() {
    return [{ tag: "figure" }];
  },
  renderHTML({ node, HTMLAttributes }) {
    return ["figure", mergeAttributes(HTMLAttributes), ["figcaption", {}, node.attrs.caption]];
  },
  addNodeView() {
    return ReactNodeViewRenderer(FigureView, {
      // A picture in a row: the element around the view sits beside its neighbours.
      attrs: ({ node }): Record<string, string> =>
        node.attrs.row ? { "data-in-row": "", style: `width: calc(${node.attrs.width ?? 48}% - 1%)` } : { style: "" },
    });
  },
});

function FigureView({ node, updateAttributes, deleteNode, selected }: ReactNodeViewProps) {
  const ctx = useContext(DocCtx)!;
  const { ref, asset, shot, want, fail, caption, width, bid, row } = node.attrs as {
    row: string | null;
    ref: string | null;
    asset: string | null;
    shot: string | null;
    want: string | null;
    fail: string | null;
    caption: string;
    width: number | null;
    bid: string | null;
  };
  const box = useRef<HTMLDivElement>(null);
  /** Width while dragging the corner handle. */
  const [dragW, setDragW] = useState<number | null>(null);
  const startDrag = (e: React.PointerEvent) => {
    e.preventDefault();
    const img = box.current?.querySelector("img");
    // Against the page, also for a picture in a row (its own box is only part of the page).
    const page = box.current?.closest(".ProseMirror") ?? box.current?.parentElement;
    if (!img || !page) return;
    const full = page.getBoundingClientRect().width;
    const start = img.getBoundingClientRect().width;
    const x0 = e.clientX;
    let last = width;
    const move = (ev: PointerEvent) => {
      last = Math.round(Math.min(100, Math.max(10, ((start + ev.clientX - x0) / full) * 100)));
      setDragW(last);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setDragW(null);
      updateAttributes({ width: last });
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };
  const shown = dragW ?? width;
  const pic = ctx.resolve(ref, asset, shot);
  const input = useRef<HTMLInputElement>(null);
  const [showOld, setShowOld] = useState(false);
  const [err, setErr] = useState("");
  return (
    <NodeViewWrapper
      className={`doc-figure ${selected ? "sel" : ""} ${row ? "in-row" : ""}`}
      data-ref={ref ?? undefined}
    >
      <div className="fig-tools" contentEditable={false}>
        <span className={`fig-badge ${pic.kind}`}>{pic.label}</span>
        <span className="spacer" />
        {ref && (
          <button className="btn small" onClick={() => ctx.gotoReference(ref)} title="기존 매뉴얼 탭의 같은 장면으로 갑니다">
            기존 매뉴얼의 같은 그림
          </button>
        )}
        {pic.old && (
          <button className="btn small" onClick={() => setShowOld(!showOld)}>
            {showOld ? "예전 그림 닫기" : "예전 그림"}
          </button>
        )}
        {want && bid && (
          <button className="btn small" onClick={() => ctx.shootSlot(bid)} title={`찍을 장면: ${want}`}>
            {shot ? "다시 찍기" : "이 장면 찍기"}
          </button>
        )}
        <button className="btn small" onClick={() => input.current?.click()}>
          {pic.kind === "none" ? "그림 올리기" : "그림 바꾸기"}
        </button>
        {asset && (ref || shot) && (
          <button className="btn small" onClick={() => updateAttributes({ asset: null })} title="올린 그림을 빼고 자동 그림으로 되돌립니다">
            자동 그림으로
          </button>
        )}
        <span className="fig-sizes">
          {[
            { w: 33, label: "작게" },
            { w: 60, label: "중간" },
            { w: 100, label: "크게" },
            { w: null, label: "원래" },
          ].map((o) => (
            <button key={o.label} className={`btn small ${width === o.w ? "on" : ""}`} onClick={() => updateAttributes({ width: o.w })} title={o.w ? `페이지 폭의 ${o.w}%` : "그림 원래 크기 (페이지보다 크면 페이지 폭)"}>
              {o.label}
            </button>
          ))}
        </span>
        <button className="btn small" onClick={deleteNode}>삭제</button>
        <input
          ref={input}
          type="file"
          accept="image/png,image/jpeg,image/gif,image/webp"
          hidden
          onChange={async (e) => {
            const f = e.target.files?.[0];
            e.target.value = "";
            if (!f) return;
            try {
              setErr("");
              // An old-manual picture's upload belongs to its scene, so ② shows it too.
              if (ref && !asset) await ctx.uploadScene(ref, f);
              else updateAttributes({ asset: await ctx.upload(f) });
            } catch (x) {
              setErr(String((x as Error).message ?? x));
            }
          }}
        />
      </div>
      {err && <p className="flash err small">{err}</p>}
      <div className={`fig-pair ${showOld ? "two" : ""}`} contentEditable={false}>
        {pic.src ? (
          <div className="fig-img" ref={box} style={shown && !row ? { width: `${shown}%` } : undefined}>
            <img src={pic.src} alt={caption} />
            <span className="fig-handle" onPointerDown={startDrag} title="끌어서 크기 조절" />
            {dragW != null && <span className="fig-size-tip">{dragW}%</span>}
          </div>
        ) : (
          <div className="fig-empty">
            {want ? (
              <>
                <b>찍을 장면</b> {want}
                {fail ? <div className="fig-fail">찍지 못함: {fail}</div> : <div className="muted small">아직 찍지 않았습니다. "이 장면 찍기"를 누르거나 그림을 올리세요.</div>}
              </>
            ) : (
              "그림이 없습니다. 올려 주세요."
            )}
          </div>
        )}
        {showOld && pic.old && <img src={pic.old} alt="예전 그림" className="fig-old" />}
      </div>
      <input
        className="fig-caption"
        value={caption}
        placeholder="캡션 (선택)"
        onChange={(e) => updateAttributes({ caption: e.target.value })}
      />
    </NodeViewWrapper>
  );
}

interface Props {
  featureKey: string;
  /** Bumped after commands finish (document import, scene capture). */
  version: number;
  /** Picture to scroll to (from the old manual tab). */
  focus: { image: string; n: number } | null;
  onGotoReference: (image: string) => void;
  onGoMake: () => void;
  /** This step is on screen. */
  active: boolean;
  onCommand: (name: string, arg?: string) => void;
  /** Menu group of the feature, for where it is published. */
  group: string | null;
}



export default function DocEditor({ featureKey, version, focus, onGotoReference, onGoMake, active, onCommand, group }: Props) {
  const [doc, setDoc] = useState<ManualDoc | null | undefined>(undefined);
  /** Bumped when a document is (re)loaded, not on save, so the editor keeps its undo history. */
  const [loaded, setLoaded] = useState(0);
  const [shots, setShots] = useState<SceneShots | null>(null);
  const [refId, setRefId] = useState<string | null>(null);
  /** Old pictures that are not scenes to capture (crops, icons, diagrams). */
  const [noCapture, setNoCapture] = useState<Set<string>>(new Set());
  const [dirty, setDirty] = useState(false);
  /** HTML as loaded or last saved; `dirty` compares against it. */
  const baseline = useRef<{ editor: Editor | null; html: string }>({ editor: null, html: "" });
  const [linking, setLinking] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [publishing, setPublishing] = useState(false);
  const [confluenceOn, setConfluenceOn] = useState(false);
  const [versions, setVersions] = useState<{ version: number; updated_at: number }[]>([]);
  /** Version to compare with; null = editing. */
  const [against, setAgainst] = useState<number | null>(null);
  const [baseHtml, setBaseHtml] = useState<string | null>(null);
  const [proposals, setProposals] = useState<Proposal[]>([]);
  /** Why blocks changed since the last save; sent with the next save. */
  const [pending, setPending] = useState<DocChange[]>([]);
  const [prov, setProv] = useState<Provenance>({ blocks: {}, log: [] });
  const [grounding, setGrounding] = useState<Grounding>({ version: 0, at: 0, blocks: {} });
  const [facts, setFacts] = useState<Map<string, ManualFact>>(new Map());
  const [panelTab, setPanelTab] = useState<PanelTab>("요청");
  const [askSignal, setAskSignal] = useState<{ text?: string; n: number } | null>(null);
  const [basisOpen, setBasisOpen] = useState<{ bid: string; n: number } | null>(null);
  /** A request's result shown large in the main area. */
  const [preview, setPreview] = useState<{ older: string; newer: string } | null>(null);
  const previewBox = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (preview) previewBox.current?.scrollIntoView({ behavior: "smooth", block: "start" });
  }, [preview]);
  const [flash, setFlash] = useState<{ ok: boolean; text: string } | null>(null);
  const picInput = useRef<HTMLInputElement>(null);

  /** Feature whose document is loaded: later command results must not wipe unsaved edits. */
  const loadedFor = useRef<string | null>(null);
  const hasDoc = !!doc;
  const dirtyRef = useRef(false);
  dirtyRef.current = dirty;
  const docVersion = useRef<number | null>(null);
  docVersion.current = doc?.version ?? null;
  /** The document changed on the engine (e.g. "매뉴얼 쓰기") while this one has unsaved edits. */
  const [newer, setNewer] = useState<number | null>(null);
  const reload = () => {
    setNewer(null);
    setDirty(false);
    setPending([]);
    fetchDoc(featureKey)
      .then((d) => {
        setDoc(d);
        setLoaded((n) => n + 1);
      })
      .catch(() => {});
  };
  useEffect(() => {
    let live = true;
    const switched = loadedFor.current !== featureKey || !hasDoc;
    loadedFor.current = featureKey;
    if (switched) {
      setDoc(undefined);
      setDirty(false);
      setNewer(null);
    }
    const got = (d: ManualDoc | null) => {
      if (!live) return;
      setDoc(d);
      setLoaded((n) => n + 1);
    };
    if (switched) fetchDoc(featureKey).then(got).catch(() => got(null));
    else
      // A job may have written a new version: take it, unless there are edits to keep.
      fetchDoc(featureKey)
        .then((d) => {
          if (!live || !d || d.version === docVersion.current) return;
          if (dirtyRef.current) setNewer(d.version);
          else got(d);
        })
        .catch(() => {});
    fetchDocVersions(featureKey).then((v) => live && setVersions(v)).catch(() => live && setVersions([]));
    fetchProposals(featureKey).then((p) => live && setProposals(p.items)).catch(() => live && setProposals([]));
    fetchProvenance(featureKey).then((p) => live && setProv(p)).catch(() => {});
    fetchGrounding(featureKey).then((g) => live && setGrounding(g)).catch(() => {});
    fetchConfluence(featureKey).then((c) => live && setConfluenceOn(!!c.enabled)).catch(() => {});
    fetchManual(featureKey)
      .then((r) => live && setFacts(new Map((r?.facts ?? []).map((f) => [f.id, f]))))
      .catch(() => {});
    fetchSceneShots(featureKey).then((s) => live && setShots(s)).catch(() => live && setShots(null));
    fetchReferences()
      .then(async (l) => {
        const id = l.items.find((d) => d.feature === featureKey && !d.ignored)?.id ?? null;
        if (!live) return;
        setRefId(id);
        const r = id ? await fetchReference(id) : null;
        if (live && r) setNoCapture(new Set(r.images.filter((i) => i.scene && !i.scene.capture).map((i) => i.file)));
      })
      .catch(() => live && setRefId(null));
    return () => {
      live = false;
    };
  }, [featureKey, version]);

  // Picture choices and captures change in ②: read them again whenever this step is shown.
  useEffect(() => {
    if (!active) return;
    let live = true;
    fetchSceneShots(featureKey).then((s) => live && setShots(s)).catch(() => {});
    fetchProposals(featureKey).then((p) => live && setProposals(p.items)).catch(() => {});
    return () => {
      live = false;
    };
  }, [active, featureKey]);

  const upload = (f: File) => uploadDocAsset(featureKey, f);
  const uploadScene = async (ref: string, f: File) => setShots(await setSceneUpload(featureKey, ref, f));
  const reference = doc?.reference ?? refId;

  const resolve: Ctx["resolve"] = pictureResolver(featureKey, reference, shots, noCapture);

  /** Picture slots of the saved document still without a picture. */
  const slotsLeft = useMemo(
    () =>
      doc
        ? Array.from(new DOMParser().parseFromString(doc.html, "text/html").body.querySelectorAll(":scope > figure[data-want]")).filter(
            (f) => !f.hasAttribute("data-shot") && !f.hasAttribute("data-asset"),
          ).length
        : 0,
    [doc],
  );
  // The capture writes into the saved document, so unsaved edits would conflict.
  const shootSlot = (bid: string | null) => {
    if (dirty) return setFlash({ ok: false, text: "저장하지 않은 편집이 있습니다. 먼저 저장한 뒤 찍어 주세요." });
    onCommand("shotdoc", bid ? `${featureKey}|${bid}` : featureKey);
  };

  const insertPictures = async (ed: Editor, files: File[], pos?: number) => {
    for (const f of files.filter((x) => x.type.startsWith("image/"))) {
      try {
        const asset = await upload(f);
        const node = { type: "figure", attrs: { asset, caption: "" } };
        if (pos != null) ed.chain().focus().insertContentAt(pos, node).run();
        else ed.chain().focus().insertContent(node).run();
      } catch (x) {
        setFlash({ ok: false, text: String((x as Error).message ?? x) });
      }
    }
  };

  const editor = useEditor(
    {
      extensions: [
        StarterKit.configure({ link: { openOnClick: false, autolink: true } }),
        Table.configure({ resizable: false }),
        TableRow,
        TableHeader,
        TableCell,
        TextStyle,
        Color,
        TextAlign.configure({ types: ["heading", "paragraph"] }),
        TaskList,
        TaskItem.configure({ nested: true }),
        Details.configure({ persist: true, HTMLAttributes: { class: "expand" } }),
        DetailsSummary,
        DetailsContent,
        Placeholder.configure({
          placeholder: ({ node }) =>
            node.type.name === "detailsSummary" ? "펼치기 제목" : "내용을 입력하세요. '/'를 입력하면 표, 패널, 그림 등을 넣을 수 있습니다",
        }),
        Panel,
        Status,
        Figure,
        BlockIds,
        OutlineLook,
      ],
      content: doc?.html ?? "",
      editable: !!doc,
      // Only real changes count: plugins tidy the document on load (e.g. tables) without the
      // person changing anything.
      onCreate: ({ editor: e }) => {
        baseline.current = { editor: e, html: e.getHTML() };
        // Load-time tidying runs right after creation; take the settled document as the baseline.
        setTimeout(() => {
          if (baseline.current.editor !== e) return;
          baseline.current = { editor: e, html: e.getHTML() };
          setDirty(false);
        }, 0);
      },
      onUpdate: ({ editor: e, transaction }) => {
        if (!transaction.docChanged) return;
        // The editor is re-created when the document arrives; compare each one with its own start.
        if (baseline.current.editor !== e) baseline.current = { editor: e, html: e.getHTML() };
        setDirty(e.getHTML() !== baseline.current.html);
      },
      editorProps: {
        attributes: { class: "doc-body" },
        handlePaste: (_view, e) => {
          const files = Array.from(e.clipboardData?.files ?? []);
          if (!files.some((f) => f.type.startsWith("image/")) || !editor) return false;
          void insertPictures(editor, files);
          return true;
        },
        handleDrop: (view, e) => {
          const files = Array.from(e.dataTransfer?.files ?? []);
          if (!files.some((f) => f.type.startsWith("image/")) || !editor) return false;
          const pos = view.posAtCoords({ left: e.clientX, top: e.clientY })?.pos;
          void insertPictures(editor, files, pos);
          return true;
        },
      },
    },
    [loaded],
  );

  useEffect(() => {
    if (against == null) return setBaseHtml(null);
    let live = true;
    fetchDocVersion(featureKey, against)
      .then((d) => live && setBaseHtml(d?.html ?? ""))
      .catch(() => live && setBaseHtml(""));
    return () => {
      live = false;
    };
  }, [featureKey, against]);

  const settle = async (p: Proposal, accept: boolean) => {
    if (accept && editor) {
      const flat = (t: string) => t.replace(/\s+/g, "");
      const tops: { from: number; to: number; text: string }[] = [];
      editor.state.doc.forEach((node, offset) => tops.push({ from: offset, to: offset + node.nodeSize, text: flat(node.textContent) }));
      const byBid: { from: number; to: number; bid: string | null }[] = [];
      editor.state.doc.forEach((node, offset) => byBid.push({ from: offset, to: offset + node.nodeSize, bid: node.attrs.bid ?? null }));
      const target =
        (p.bid && byBid.find((t) => t.bid === p.bid)) ?? tops.find((t) => t.text === flat(p.before)) ?? tops[p.block];
      if (!target) {
        setFlash({ ok: false, text: "제안이 가리키는 문단을 찾지 못했습니다. 직접 고쳐 주세요." });
        return;
      }
      const old = byBid.find((t) => t.from === target.from)?.bid ?? null;
      const next = withBids(p.html);
      const range = p.action === "replace" ? { from: target.from, to: target.to } : target.to;
      editor.chain().focus().insertContentAt(range, next.html).run();
      setPending((all) => [
        ...all,
        {
          bids: next.bids,
          kind: "proposal",
          summary: p.reason,
          facts: p.facts,
          inherit: p.action === "replace" && old ? [old] : [],
        },
      ]);
    }
    await resolveProposal(featureKey, p.id);
    setProposals((all) => all.filter((x) => x.id !== p.id));
  };

  // Scroll to a picture asked for from the old manual tab; say so when the new manual lacks it.
  useEffect(() => {
    if (!focus || !doc || !editor) return;
    setAgainst(null);
    const t = setTimeout(() => {
      const el = document.querySelector(`.doc-figure[data-ref="${CSS.escape(focus.image)}"]`);
      if (!el) {
        setFlash({ ok: false, text: "새 매뉴얼에는 이 그림이 없습니다. 문서에서 지웠거나 아직 문서로 옮기지 않았습니다." });
        return;
      }
      el.scrollIntoView({ behavior: "smooth", block: "center" });
      el.classList.add("flash-focus");
      setTimeout(() => el.classList.remove("flash-focus"), 1800);
    }, 100);
    return () => clearTimeout(t);
  }, [focus, doc, editor]);

  /** Joined tables are remembered as a reasoned change for the next save. */
  const recordJoins = (pairs: SplitPair[]) => {
    setPending((all) => [
      ...all,
      ...pairs.map((p) => ({
        bids: [p.first.attrs.bid as string].filter(Boolean),
        kind: "edit",
        summary: "쪽에 걸쳐 잘린 표를 하나로 합침",
        facts: [],
        inherit: [p.second.attrs.bid as string].filter(Boolean),
      })),
    ]);
    if (pairs.length) setFlash({ ok: true, text: `잘린 표 ${pairs.length}곳을 합쳤습니다. 저장하면 버전에 남습니다.` });
  };
  const splitCount = useEditorState({ editor, selector: ({ editor: e }) => (e ? splitTables(e).length : 0) }) ?? 0;

  const ctxMenu = useContextMenu(editor, {
    ask: (text) => {
      setPanelTab("요청");
      setAskSignal((s) => ({ text, n: (s?.n ?? 0) + 1 }));
    },
    basis: () => {
      const bid = editor && focusOf(editor)?.bids[0];
      if (bid) setBasisOpen((s) => ({ bid, n: (s?.n ?? 0) + 1 }));
    },
  });
  /** Moves the cursor to a block and shows it. */
  const gotoBid = (bid: string) => {
    if (!editor) return;
    let at: number | null = null;
    editor.state.doc.forEach((node, offset) => {
      if (at == null && node.attrs.bid === bid) at = offset;
    });
    if (at == null) return setFlash({ ok: false, text: "그 부분은 지금 문서에 없습니다." });
    editor.chain().focus().setTextSelection(at + 1).run();
    (editor.view.nodeDOM(at) as HTMLElement | null)?.scrollIntoView({ behavior: "smooth", block: "center" });
  };

  const save = async () => {
    if (!editor) return;
    try {
      const saved = editor.getHTML();
      const d = await saveDoc(featureKey, saved, pending, doc?.version);
      baseline.current = { editor, html: saved };
      setDoc(d);
      setDirty(false);
      setPending([]);
      fetchProvenance(featureKey).then(setProv).catch(() => {});
      fetchDocVersions(featureKey).then(setVersions).catch(() => {});
      setFlash({ ok: true, text: `저장했습니다 (버전 ${d.version})` });
    } catch (x) {
      setFlash({ ok: false, text: `저장하지 못했습니다: ${(x as Error).message}` });
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "s" && doc) {
        e.preventDefault();
        void save();
      }
      if ((e.ctrlKey || e.metaKey) && e.key === "k" && editor?.isFocused) {
        e.preventDefault();
        setLinking(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (doc === undefined) return <p className="muted">불러오는 중…</p>;

  if (doc === null) {
    return (
      <div className="doc-empty">
        <p>아직 매뉴얼 문서가 없습니다.</p>
        <button className="btn primary" onClick={onGoMake}>③ 매뉴얼 만들기로 →</button>
      </div>
    );
  }

  const pics = shots?.scenes ?? [];
  const current = pics.filter((s) => s.file && s.status !== "failed").length;

  return (
    <DocCtx.Provider value={{ resolve, upload, uploadScene, gotoReference: onGotoReference, shootSlot }}>
      <div className="doced">
        <div className="doc-head">
          <b>{doc.title}</b>
          <span className="muted small">
            버전 {doc.version}
            {doc.origin !== "generated" ? " · 기존 매뉴얼에서 옮김" : " · 코드로 새로 씀"}
            {reference && ` · 현재 화면 그림 ${current}장`}
          </span>
          <span className="spacer" />
          <select
            value={against ?? ""}
            onChange={(e) => setAgainst(e.target.value === "" ? null : Number(e.target.value))}
            title="이전 매뉴얼과 비교해 바뀐 곳을 봅니다"
          >
            <option value="">비교 안 함 (편집)</option>
            {versions
              .filter((v) => v.version !== doc.version || dirty)
              .map((v) => (
                <option key={v.version} value={v.version}>
                  {v.version === (doc.base ?? (doc.origin !== "generated" ? 0 : -1)) ? `기존 매뉴얼(버전 ${v.version})과 비교` : `버전 ${v.version}과 비교`}
                  {v.updated_at ? ` · ${new Date(v.updated_at * 1000).toLocaleString("ko-KR", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}` : ""}
                </option>
              ))}
          </select>
          {against != null && against !== doc.version && (
            <button
              className="btn"
              title="지금 문서는 버전으로 남고, 고른 버전이 새 현재 버전이 됩니다"
              onClick={async () => {
                if (dirty && !window.confirm("저장하지 않은 편집이 사라집니다. 되돌릴까요?")) return;
                try {
                  const d = await restoreDoc(featureKey, against);
                  setDoc(d);
                  setAgainst(null);
                  setDirty(false);
                  setLoaded((n) => n + 1);
                  fetchDocVersions(featureKey).then(setVersions).catch(() => {});
                  setFlash({ ok: true, text: `버전 ${against}의 내용으로 되돌렸습니다 (새 버전 ${d.version})` });
                } catch (x) {
                  setFlash({ ok: false, text: String((x as Error).message ?? x) });
                }
              }}
            >
              이 버전으로 되돌리기
            </button>
          )}
          {splitCount > 0 && editor && (
            <button className="btn" onClick={() => recordJoins(joinTables(editor, splitTables(editor)))} title="나란히 붙어 있고 열 수가 같은 표(쪽 경계에서 잘린 표)를 하나로 합칩니다">
              잘린 표 합치기 ({splitCount}곳)
            </button>
          )}
          <button
            className="btn"
            disabled={exporting}
            title="Confluence Cloud에서 '더 보기 → 템플릿 및 가져오기 → 가져오기 → Word(.docx)'로 올리면 페이지가 됩니다. 그림·표·목록·패널 포함."
            onClick={async () => {
              if (!editor) return;
              setExporting(true);
              try {
                // The Word writer is large; load it only when used.
                const { exportDocx } = await import("../exportDocx");
                await exportDocx(doc.title, editor.getHTML(), (fig) => resolve(fig.dataset.ref ?? null, fig.dataset.asset ?? null, fig.dataset.shot ?? null).src);
                setFlash({
                  ok: true,
                  text: `${doc.title}.docx를 내려받았습니다${dirty ? " (저장 안 한 편집 포함)" : ""}. 컨플루언스에서 상위 페이지를 열고 '더 보기(…) → 템플릿 및 가져오기 → 가져오기 → Word'로 이 파일을 올리세요.`,
                });
              } catch (x) {
                setFlash({ ok: false, text: `내보내지 못했습니다: ${(x as Error).message}` });
              } finally {
                setExporting(false);
              }
            }}
          >
            {exporting ? "만드는 중…" : "Word로 내보내기"}
          </button>
          {slotsLeft > 0 && (
            <button className="btn" onClick={() => shootSlot(null)} title="그림 자리마다 적힌 장면을 지금 제품 화면에서 찍어 넣습니다 (쓰기 요청은 막힘)">
              그림 자리 찍기 ({slotsLeft})
            </button>
          )}
          {confluenceOn && (
            <button className="btn" onClick={() => setPublishing(true)} title="컨플루언스 Cloud에 실제 페이지로 올립니다 (패널·상태·그림 그대로, 모델 사용 없음)">
              컨플루언스에 게시
            </button>
          )}
          {publishing && editor && (
            <ConfluencePublish
              featureKey={featureKey}
              title={doc.title}
              group={group}
              html={() => editor.getHTML()}
              srcOf={(fig) => resolve(fig.dataset.ref ?? null, fig.dataset.asset ?? null, fig.dataset.shot ?? null).src}
              dirty={dirty}
              onClose={() => setPublishing(false)}
            />
          )}
          <button
            className="btn"
            onClick={() => (dirty ? setFlash({ ok: false, text: "저장한 뒤에 근거를 확인하세요 (저장된 문서를 확인합니다)." }) : onCommand("docground", featureKey))}
            title="문단마다 코드 사실과 대조해 왜 이렇게 썼는지(근거)를 확인합니다. 결과는 문단 옆 ?에 나옵니다."
          >
            근거 확인
            {grounding.at ? ` · ${new Date(grounding.at * 1000).toLocaleString("ko-KR", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}` : ""}
          </button>
          <button className="btn primary" disabled={!dirty} onClick={save} title="Ctrl+S">
            {dirty ? "저장" : "저장됨"}
          </button>
        </div>
        {flash && <p className={`flash ${flash.ok ? "ok" : "err"}`}>{flash.text}</p>}
        {newer != null && (
          <p className="flash err">
            엔진에서 이 문서가 바뀌었습니다 (지금 버전 {newer}, 예: 매뉴얼 쓰기). 보고 있는 것은 버전 {doc.version}이고 저장하지 않은 편집이 있습니다.{" "}
            <button className="btn small" onClick={reload}>새 버전 불러오기 (편집 버림)</button>
          </p>
        )}
        <div className="doced-cols">
          <div className="doced-main">
            {preview && (
              <div className="preview-big" ref={previewBox}>
                <div className="preview-head">
                  <b>요청 결과 미리보기</b>
                  <span className="muted small">적용은 오른쪽 패널에서 합니다</span>
                  <span className="spacer" />
                  <button className="btn small" onClick={() => setPreview(null)}>닫기</button>
                </div>
                <DiffView older={preview.older} newer={preview.newer} resolve={resolve} label="지금" newLabel="바뀔 내용" />
              </div>
            )}
            {proposals.length > 0 && (
              <div className="proposals">
                <b>코드 기준 갱신 제안 {proposals.length}개</b>
                <span className="muted small"> · 수락하면 문서에 반영되고, 저장해야 남습니다</span>
                {proposals.map((p) => (
                  <div key={p.id} className="proposal">
                    <div className="proposal-head">
                      <span className={`badge ${p.action === "replace" ? "warn" : "ok"}`}>{p.action === "replace" ? "바꾸기" : "추가"}</span>
                      <span>{p.reason}</span>
                      <span className="spacer" />
                      <button className="btn small primary" onClick={() => settle(p, true)} disabled={against != null}>수락</button>
                      <button className="btn small" onClick={() => settle(p, false)}>거절</button>
                    </div>
                    {p.action === "replace" && <p className="proposal-before">{p.before}</p>}
                    <div className="proposal-after doc-body" dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(p.html) }} />
                  </div>
                ))}
              </div>
            )}
            {against != null && baseHtml != null && editor && (
              <DiffView older={baseHtml} newer={editor.getHTML()} resolve={resolve} label={against === 0 && doc.origin !== "generated" ? "기존 매뉴얼" : `버전 ${against}`} />
            )}
            {against == null && editor && (
              <>
                <Toolbar editor={editor} onPicture={() => picInput.current?.click()} onLink={() => setLinking(true)} />
                <SlashMenu editor={editor} onPicture={() => picInput.current?.click()} />
                <SelectionBubble editor={editor} linking={linking} setLinking={setLinking} />
                <TableBubble
              editor={editor}
              onJoinBelow={() => {
                const p = pairBelowCursor(editor);
                if (p) recordJoins(joinTables(editor, [p]));
              }}
            />
                <PanelBubble editor={editor} />
              </>
            )}
            <input
              ref={picInput}
              type="file"
              accept="image/png,image/jpeg,image/gif,image/webp"
              multiple
              hidden
              onChange={(e) => {
                const files = Array.from(e.target.files ?? []);
                e.target.value = "";
                if (editor) void insertPictures(editor, files);
              }}
            />
            <div className="doc-page" hidden={against != null || !!preview}>
              <h1 className="doc-title">{doc.title}</h1>
              <div className="doc-with-basis" onContextMenu={ctxMenu.onContextMenu}>
                <EditorContent editor={editor} />
                {editor && <BasisGutter
                    editor={editor}
                    prov={prov}
                    grounding={grounding}
                    pending={pending}
                    facts={facts}
                    refId={reference}
                    open={basisOpen}
                  />}
              </div>
              {ctxMenu.menu}
            </div>
          </div>
          {editor && (
            <DocSidePanel
              onPreview={(older, newer) => setPreview({ older, newer })}
              editor={editor}
              featureKey={featureKey}
              prov={prov}
              facts={facts}
              pending={pending}
              onChange={(c) => setPending((all) => [...all, c])}
              tab={panelTab}
              setTab={setPanelTab}
              ask={askSignal}
              renderDiff={(older, newer) => <DiffView older={older} newer={newer} resolve={resolve} label="지금" compact />}
              onGotoBid={gotoBid}
            />
          )}
        </div>
      </div>
    </DocCtx.Provider>
  );
}

interface Op {
  kind: "same" | "added" | "removed" | "changed";
  html: string;
  old?: string;
}

const topBlocks = (html: string) => Array.from(new DOMParser().parseFromString(html, "text/html").body.children);
const keyOf = (el: Element) =>
  el.tagName === "FIGURE"
    ? `fig:${el.getAttribute("data-ref") ?? el.getAttribute("data-asset") ?? el.getAttribute("data-shot")}`
    : (el.textContent ?? "").replace(/\s+/g, "");

/** Block-level diff (LCS on text); an unmatched old block next to a new one is "changed". */
function diffBlocks(older: string, newer: string): Op[] {
  const a = topBlocks(older);
  const b = topBlocks(newer);
  const ka = a.map(keyOf);
  const kb = b.map(keyOf);
  const L = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--) L[i][j] = ka[i] === kb[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
  const ops: Op[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && ka[i] === kb[j]) {
      ops.push({ kind: "same", html: b[j].outerHTML });
      i++;
      j++;
    } else if (j < b.length && (i >= a.length || L[i][j + 1] >= L[i + 1][j])) {
      ops.push({ kind: "added", html: b[j++].outerHTML });
    } else {
      ops.push({ kind: "removed", html: a[i++].outerHTML });
    }
  }
  // removed followed by added (or the reverse) at the same spot reads as one change
  const out: Op[] = [];
  for (const op of ops) {
    const prev = out[out.length - 1];
    const pair = prev && !op.html.startsWith("<figure") && !prev.html.startsWith("<figure");
    if (pair && prev.kind === "removed" && op.kind === "added") {
      out[out.length - 1] = { kind: "changed", html: op.html, old: prev.html };
    } else if (pair && prev.kind === "added" && op.kind === "removed") {
      out[out.length - 1] = { kind: "changed", html: prev.html, old: op.html };
    } else out.push(op);
  }
  return out;
}

function DiffBlock({ html, resolve }: { html: string; resolve: Ctx["resolve"] }) {
  if (html.startsWith("<figure")) {
    const el = topBlocks(html)[0];
    const pic = resolve(el.getAttribute("data-ref"), el.getAttribute("data-asset"), el.getAttribute("data-shot"));
    return (
      <div className="diff-fig">
        <span className={`fig-badge ${pic.kind}`}>{pic.label}</span>
        {pic.src && <img src={pic.src} alt="" loading="lazy" />}
        {el.querySelector("figcaption")?.textContent && <p className="muted small">{el.querySelector("figcaption")!.textContent}</p>}
      </div>
    );
  }
  return <div dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(html) }} />;
}

/** Word-level diff of two texts: runs of [kind, text] where kind is same | add | del. */
function wordDiff(a: string, b: string): [string, string][] {
  const A = a.split(/(\s+)/).filter(Boolean);
  const B = b.split(/(\s+)/).filter(Boolean);
  if (A.length * B.length > 250000) return [["del", a], ["add", b]];
  const L = Array.from({ length: A.length + 1 }, () => new Array<number>(B.length + 1).fill(0));
  for (let i = A.length - 1; i >= 0; i--)
    for (let j = B.length - 1; j >= 0; j--) L[i][j] = A[i] === B[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
  const out: [string, string][] = [];
  const add = (k: string, t: string) => {
    const last = out[out.length - 1];
    if (last && last[0] === k) last[1] += t;
    else out.push([k, t]);
  };
  let i = 0;
  let j = 0;
  while (i < A.length || j < B.length) {
    if (i < A.length && j < B.length && A[i] === B[j]) {
      add("same", A[i]);
      i++;
      j++;
    } else if (j < B.length && (i >= A.length || L[i][j + 1] >= L[i + 1][j])) add("add", B[j++]);
    else add("del", A[i++]);
  }
  return out;
}

const TEXTY = new Set(["P", "H1", "H2", "H3", "BLOCKQUOTE"]);

/** A changed text block with the changed words marked: deletions on the old side, additions on the new. */
function WordBlock({ older, newer, side }: { older: string; newer: string; side: "old" | "new" }) {
  const el = topBlocks(side === "old" ? older : newer)[0];
  const Tag = el.tagName.toLowerCase() as "p";
  const runs = wordDiff(topBlocks(older)[0].textContent ?? "", topBlocks(newer)[0].textContent ?? "");
  return (
    <Tag>
      {runs.map(([k, t], i) =>
        k === "same" ? <span key={i}>{t}</span> : k === (side === "old" ? "del" : "add") ? <mark key={i} className={k}>{t}</mark> : null,
      )}
    </Tag>
  );
}

/** Two documents side by side, rows aligned, with what changed marked on both sides. */
function DiffView({
  older,
  newer,
  resolve,
  label,
  newLabel = "지금 (저장 안 한 편집 포함)",
  compact,
}: {
  older: string;
  newer: string;
  resolve: Ctx["resolve"];
  label: string;
  newLabel?: string;
  compact?: boolean;
}) {
  const ops = diffBlocks(older, newer);
  const count = (k: Op["kind"]) => ops.filter((o) => o.kind === k).length;
  const [onlyChanges, setOnlyChanges] = useState(!!compact);
  const first = useRef<HTMLDivElement>(null);
  const changes = ops.filter((o) => o.kind !== "same").length;
  return (
    <div className={`diff2 ${compact ? "compact" : ""}`}>
      {!compact && (
        <div className="diff-summary">
          <b>
            {label} ↔ {newLabel}
          </b>
          <span className="badge ok">추가 {count("added")}</span>
          <span className="badge warn">바뀜 {count("changed")}</span>
          <span className="badge err">빠짐 {count("removed")}</span>
          <span className="spacer" />
          {changes > 0 && (
            <button className="btn small" onClick={() => first.current?.scrollIntoView({ behavior: "smooth", block: "center" })}>
              첫 변경으로
            </button>
        )}
        <label className="small">
          <input type="checkbox" checked={onlyChanges} onChange={(e) => setOnlyChanges(e.target.checked)} /> 바뀐 곳만
        </label>
      </div>
      )}
      <div className="diff-grid doc-body">
        <div className="diff-col-head">{label}</div>
        <div className="diff-col-head">{newLabel}</div>
        {ops.map((o, i) => {
          if (onlyChanges && o.kind === "same") return null;
          const isFirst = o.kind !== "same" && ops.findIndex((x) => x.kind !== "same") === i;
          const oldHtml = o.kind === "changed" ? o.old! : o.kind === "removed" || o.kind === "same" ? o.html : null;
          const newHtml = o.kind === "removed" ? null : o.html;
          const texty = o.kind === "changed" && TEXTY.has(topBlocks(o.old!)[0]?.tagName) && TEXTY.has(topBlocks(o.html)[0]?.tagName);
          return (
            <div key={i} className={`diff-row ${o.kind}`}>
              <div className="diff-cell old" ref={isFirst ? first : undefined}>
                {oldHtml == null ? (
                  <span className="diff-none">(없음)</span>
                ) : texty ? (
                  <WordBlock older={o.old!} newer={o.html} side="old" />
                ) : (
                  <DiffBlock html={oldHtml} resolve={resolve} />
                )}
              </div>
              <div className="diff-cell new">
                {newHtml == null ? (
                  <span className="diff-none">(빠짐)</span>
                ) : texty ? (
                  <WordBlock older={o.old!} newer={o.html} side="new" />
                ) : (
                  <DiffBlock html={newHtml} resolve={resolve} />
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
