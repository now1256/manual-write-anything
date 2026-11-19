import { useEffect, useRef, useState } from "react";
import { useEditorState, type Editor } from "@tiptap/react";
import { BubbleMenu } from "@tiptap/react/menus";
import { TextSelection } from "@tiptap/pm/state";
import { pairBelowCursor } from "./tableMerge";

/** Confluence-style editing tools around the manual editor: top toolbar, "/" menu, bubbles. */

export const PANELS = [
  { type: "info", label: "정보", icon: "ℹ" },
  { type: "note", label: "참고", icon: "✎" },
  { type: "warning", label: "주의", icon: "!" },
  { type: "success", label: "성공", icon: "✓" },
] as const;

export const STATUS = [
  { color: "grey", label: "회색" },
  { color: "blue", label: "파랑" },
  { color: "green", label: "초록" },
  { color: "yellow", label: "노랑" },
  { color: "red", label: "빨강" },
] as const;

/** Text colours from Confluence's palette. */
const COLORS = [
  { value: "", label: "기본" },
  { value: "#0055cc", label: "파랑" },
  { value: "#206a83", label: "청록" },
  { value: "#216e4e", label: "초록" },
  { value: "#a54800", label: "주황" },
  { value: "#ae2e24", label: "빨강" },
  { value: "#5e4db2", label: "보라" },
  { value: "#626f86", label: "회색" },
];

interface Item {
  label: string;
  hint: string;
  /** Extra words the "/" menu matches. */
  keys: string;
  icon: string;
  run: (e: Editor, onPicture: () => void) => void;
}

/** Everything that can be inserted, shared by the "+" menu and the "/" menu. */
const ITEMS: Item[] = [
  { label: "제목 1", hint: "큰 제목", keys: "heading h1 제목", icon: "H1", run: (e) => e.chain().focus().setHeading({ level: 1 }).run() },
  { label: "제목 2", hint: "중간 제목", keys: "heading h2 제목", icon: "H2", run: (e) => e.chain().focus().setHeading({ level: 2 }).run() },
  { label: "제목 3", hint: "작은 제목", keys: "heading h3 제목", icon: "H3", run: (e) => e.chain().focus().setHeading({ level: 3 }).run() },
  { label: "글머리 기호 목록", hint: "* 또는 - 후 띄어쓰기", keys: "bullet list 목록", icon: "•", run: (e) => e.chain().focus().toggleBulletList().run() },
  { label: "번호 매기기 목록", hint: "1. 후 띄어쓰기", keys: "number ordered list 목록 번호", icon: "1.", run: (e) => e.chain().focus().toggleOrderedList().run() },
  { label: "작업 목록", hint: "[] 후 띄어쓰기", keys: "task todo check 체크 할일 작업", icon: "☑", run: (e) => e.chain().focus().toggleTaskList().run() },
  { label: "표", hint: "3×3 표", keys: "table 표", icon: "▦", run: (e) => e.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run() },
  ...PANELS.map((p) => ({
    label: `${p.label} 패널`,
    hint: "색이 있는 안내 상자",
    keys: `panel ${p.type} 패널 ${p.label}`,
    icon: p.icon,
    run: (e: Editor) => e.chain().focus().wrapIn("panel", { type: p.type }).run(),
  })),
  {
    label: "상태",
    hint: "활성, 비활성 같은 색 라벨",
    keys: "status lozenge 상태 라벨",
    icon: "▭",
    run: (e) =>
      e
        .chain()
        .focus()
        .insertContent([{ type: "text", text: "상태", marks: [{ type: "status", attrs: { color: "blue" } }] }, { type: "text", text: " " }])
        .run(),
  },
  { label: "그림", hint: "파일 올리기 (끌어다 놓기·붙여 넣기도 됨)", keys: "image picture 그림 사진 이미지", icon: "🖼", run: (_e, onPicture) => onPicture() },
  { label: "펼치기", hint: "눌러서 펼치는 영역", keys: "expand details 펼치기 접기", icon: "▸", run: (e) => e.chain().focus().setDetails().run() },
  { label: "인용", hint: "> 후 띄어쓰기", keys: "quote 인용", icon: "❝", run: (e) => e.chain().focus().toggleBlockquote().run() },
  { label: "코드 블록", hint: "``` 입력", keys: "code 코드", icon: "{}", run: (e) => e.chain().focus().toggleCodeBlock().run() },
  { label: "구분선", hint: "--- 입력", keys: "divider rule hr 구분선 선", icon: "—", run: (e) => e.chain().focus().setHorizontalRule().run() },
];

const flat = (t: string) => t.toLowerCase().replace(/\s+/g, "");

/** Mouse down without stealing focus from the editor. */
const keep = (e: React.MouseEvent) => e.preventDefault();

function Btn({ label, title, active, onClick, wide }: { label: string; title: string; active?: boolean; onClick: () => void; wide?: boolean }) {
  return (
    <button type="button" className={`tb ${active ? "on" : ""} ${wide ? "wide" : ""}`} title={title} onMouseDown={keep} onClick={onClick}>
      {label}
    </button>
  );
}

/** A button that opens a small menu below it. */
function Drop({ label, title, children, wide }: { label: string; title: string; children: (close: () => void) => React.ReactNode; wide?: boolean }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!open) return;
    const off = (e: MouseEvent) => !box.current?.contains(e.target as Node) && setOpen(false);
    document.addEventListener("mousedown", off);
    return () => document.removeEventListener("mousedown", off);
  }, [open]);
  return (
    <span className="tb-drop" ref={box}>
      <button type="button" className={`tb ${open ? "on" : ""} ${wide ? "wide" : ""}`} title={title} onMouseDown={keep} onClick={() => setOpen(!open)}>
        {label} <span className="caret">▾</span>
      </button>
      {open && <div className="tb-menu">{children(() => setOpen(false))}</div>}
    </span>
  );
}

function MenuItem({ icon, label, hint, active, onClick }: { icon?: string; label: string; hint?: string; active?: boolean; onClick: () => void }) {
  return (
    <button type="button" className={`mi ${active ? "on" : ""}`} onMouseDown={keep} onClick={onClick}>
      {icon != null && <span className="mi-icon">{icon}</span>}
      <span className="mi-text">
        <span>{label}</span>
        {hint && <span className="mi-hint">{hint}</span>}
      </span>
    </button>
  );
}

const STYLES = [
  { key: "p", label: "일반 텍스트" },
  { key: "h1", label: "제목 1" },
  { key: "h2", label: "제목 2" },
  { key: "h3", label: "제목 3" },
] as const;

/** Top toolbar, in Confluence's order. */
export function Toolbar({ editor, onPicture, onLink }: { editor: Editor; onPicture: () => void; onLink: () => void }) {
  const s = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      style: e.isActive("heading", { level: 1 }) ? "h1" : e.isActive("heading", { level: 2 }) ? "h2" : e.isActive("heading", { level: 3 }) ? "h3" : "p",
      bold: e.isActive("bold"),
      italic: e.isActive("italic"),
      underline: e.isActive("underline"),
      strike: e.isActive("strike"),
      code: e.isActive("code"),
      bullet: e.isActive("bulletList"),
      ordered: e.isActive("orderedList"),
      task: e.isActive("taskList"),
      link: e.isActive("link"),
      align: e.isActive({ textAlign: "center" }) ? "center" : e.isActive({ textAlign: "right" }) ? "right" : "left",
      color: (e.getAttributes("textStyle").color as string | undefined) ?? "",
      undo: e.can().undo(),
      redo: e.can().redo(),
    }),
  });
  const c = () => editor.chain().focus();
  const styleLabel = STYLES.find((x) => x.key === s.style)?.label ?? "일반 텍스트";
  return (
    <div className="doc-toolbar">
      <Drop label={styleLabel} title="텍스트 스타일" wide>
        {(close) =>
          STYLES.map((x) => (
            <MenuItem
              key={x.key}
              label={x.label}
              hint={x.key === "p" ? "Ctrl+Alt+0" : `Ctrl+Alt+${x.key.slice(1)}`}
              active={s.style === x.key}
              onClick={() => {
                if (x.key === "p") c().setParagraph().run();
                else c().setHeading({ level: Number(x.key.slice(1)) as 1 | 2 | 3 }).run();
                close();
              }}
            />
          ))
        }
      </Drop>
      <span className="tb-sep" />
      <Btn label="B" title="굵게 (Ctrl+B)" active={s.bold} onClick={() => c().toggleBold().run()} />
      <Btn label="I" title="기울임꼴 (Ctrl+I)" active={s.italic} onClick={() => c().toggleItalic().run()} />
      <Drop label="···" title="텍스트 서식 더 보기">
        {(close) => (
          <>
            <MenuItem label="밑줄" hint="Ctrl+U" active={s.underline} onClick={() => (c().toggleUnderline().run(), close())} />
            <MenuItem label="취소선" hint="Ctrl+Shift+S" active={s.strike} onClick={() => (c().toggleStrike().run(), close())} />
            <MenuItem label="코드" hint="Ctrl+Shift+M" active={s.code} onClick={() => (c().toggleCode().run(), close())} />
            <MenuItem label="서식 지우기" hint="Ctrl+\" onClick={() => (c().unsetAllMarks().run(), close())} />
          </>
        )}
      </Drop>
      <Drop label={`A`} title="텍스트 색상">
        {(close) => (
          <div className="swatches">
            {COLORS.map((x) => (
              <button
                key={x.label}
                type="button"
                title={x.label}
                className={`swatch ${s.color === x.value ? "on" : ""}`}
                style={{ background: x.value || "#172b4d" }}
                onMouseDown={keep}
                onClick={() => {
                  if (x.value) c().setColor(x.value).run();
                  else c().unsetColor().run();
                  close();
                }}
              />
            ))}
          </div>
        )}
      </Drop>
      <span className="tb-sep" />
      <Btn label="•≡" title="글머리 기호 목록 (* 후 띄어쓰기)" active={s.bullet} onClick={() => c().toggleBulletList().run()} />
      <Btn label="1≡" title="번호 매기기 목록 (1. 후 띄어쓰기)" active={s.ordered} onClick={() => c().toggleOrderedList().run()} />
      <Btn label="☑" title="작업 목록 ([] 후 띄어쓰기)" active={s.task} onClick={() => c().toggleTaskList().run()} />
      <span className="tb-sep" />
      <Drop label={{ left: "⇤", center: "↔", right: "⇥" }[s.align as "left"]} title="텍스트 정렬">
        {(close) => (
          <>
            <MenuItem label="왼쪽 정렬" active={s.align === "left"} onClick={() => (c().setTextAlign("left").run(), close())} />
            <MenuItem label="가운데 정렬" active={s.align === "center"} onClick={() => (c().setTextAlign("center").run(), close())} />
            <MenuItem label="오른쪽 정렬" active={s.align === "right"} onClick={() => (c().setTextAlign("right").run(), close())} />
          </>
        )}
      </Drop>
      <span className="tb-sep" />
      <Btn label="🔗" title="링크 (Ctrl+K)" active={s.link} onClick={onLink} />
      <Btn label="🖼" title="그림 넣기 (끌어다 놓거나 붙여 넣어도 됩니다)" onClick={onPicture} />
      <Btn label="▦" title="표 넣기" onClick={() => c().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()} />
      <Drop label="＋" title="삽입 ('/'를 입력해도 됩니다)">
        {(close) =>
          ITEMS.slice(3).map((it) => (
            <MenuItem key={it.label} icon={it.icon} label={it.label} hint={it.hint} onClick={() => (it.run(editor, onPicture), close())} />
          ))
        }
      </Drop>
      <span className="spacer" />
      <Btn label="↶" title="실행 취소 (Ctrl+Z)" onClick={() => c().undo().run()} active={false} />
      <Btn label="↷" title="다시 실행 (Ctrl+Shift+Z)" onClick={() => c().redo().run()} active={false} />
    </div>
  );
}

/** Where the "/" menu is open: the "/" position and what was typed after it. */
interface Slash {
  from: number;
  query: string;
  top: number;
  left: number;
}

/**
 * Confluence's "/" quick insert. Typing "/" at the start of a line or after a space opens it;
 * typing filters, ↑↓ moves, Enter inserts, Esc closes.
 */
export function SlashMenu({ editor, onPicture }: { editor: Editor; onPicture: () => void }) {
  const [slash, setSlash] = useState<Slash | null>(null);
  const [index, setIndex] = useState(0);
  const dismissed = useRef<number | null>(null);
  const state = useRef<{ slash: Slash | null; items: Item[]; index: number }>({ slash: null, items: [], index: 0 });

  const items = slash ? ITEMS.filter((it) => flat(it.label + it.keys).includes(flat(slash.query))) : [];
  state.current = { slash, items, index: Math.min(index, Math.max(items.length - 1, 0)) };

  useEffect(() => {
    const update = () => {
      const { selection } = editor.state;
      const $from = selection.$from;
      if (!selection.empty || !$from.parent.isTextblock || $from.parent.type.name === "codeBlock") return setSlash(null);
      const before = $from.parent.textBetween(0, $from.parentOffset, undefined, "￼");
      const m = before.match(/(?:^|\s)\/([^\s/]{0,20})$/);
      if (!m) {
        dismissed.current = null;
        return setSlash(null);
      }
      const from = selection.from - m[1].length - 1;
      if (dismissed.current === from) return setSlash(null);
      const at = editor.view.coordsAtPos(from);
      setSlash((old) => {
        if (!old || old.query !== m[1]) setIndex(0);
        return { from, query: m[1], top: at.bottom + 4, left: at.left };
      });
    };
    editor.on("transaction", update);
    return () => {
      editor.off("transaction", update);
    };
  }, [editor]);

  const pick = (it: Item) => {
    const s = state.current.slash;
    if (!s) return;
    editor.chain().focus().deleteRange({ from: s.from, to: editor.state.selection.from }).run();
    it.run(editor, onPicture);
    setSlash(null);
  };

  useEffect(() => {
    // The view exists only once the editor is mounted; the next render attaches then.
    if (!editor.isInitialized) return;
    const dom = editor.view.dom;
    const onKey = (e: KeyboardEvent) => {
      const { slash: s, items: list, index: i } = state.current;
      if (!s) return;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = list.length || 1;
        setIndex((i + (e.key === "ArrowDown" ? 1 : n - 1)) % n);
      } else if ((e.key === "Enter" || e.key === "Tab") && list[i]) {
        e.preventDefault();
        e.stopPropagation();
        pick(list[i]);
      } else if (e.key === "Escape") {
        e.preventDefault();
        dismissed.current = s.from;
        setSlash(null);
      }
    };
    // Capture phase: before the editor turns Enter into a new paragraph.
    dom.addEventListener("keydown", onKey, true);
    return () => dom.removeEventListener("keydown", onKey, true);
  });

  if (!slash) return null;
  return (
    <div className="slash-menu" style={{ top: slash.top, left: slash.left }}>
      {items.length === 0 ? (
        <div className="mi-hint">"{slash.query}"에 맞는 항목이 없습니다</div>
      ) : (
        items.map((it, i) => (
          <MenuItem key={it.label} icon={it.icon} label={it.label} hint={it.hint} active={i === state.current.index} onClick={() => pick(it)} />
        ))
      )}
    </div>
  );
}

/** Formatting bubble over selected text, plus link editing. */
export function SelectionBubble({ editor, linking, setLinking }: { editor: Editor; linking: boolean; setLinking: (v: boolean) => void }) {
  const [href, setHref] = useState("");
  useEffect(() => {
    if (linking) setHref((editor.getAttributes("link").href as string | undefined) ?? "https://");
  }, [linking, editor]);
  const c = () => editor.chain().focus();
  const s = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      bold: e.isActive("bold"),
      italic: e.isActive("italic"),
      underline: e.isActive("underline"),
      strike: e.isActive("strike"),
      code: e.isActive("code"),
      link: e.isActive("link"),
      status: e.isActive("status"),
    }),
  });
  const apply = () => {
    const url = href.trim();
    if (!url || url === "https://") c().extendMarkRange("link").unsetLink().run();
    else c().extendMarkRange("link").setLink({ href: url }).run();
    setLinking(false);
  };
  return (
    <BubbleMenu
      editor={editor}
      pluginKey="selection"
      shouldShow={({ editor: e, state }) =>
        linking || (!state.selection.empty && !e.isActive("figure") && !e.isActive("table") && state.selection instanceof TextSelection)
      }
      options={{ placement: "top-start", offset: 8 }}
      className="bubble"
    >
      {linking ? (
        <span className="bubble-link">
          <input
            autoFocus
            value={href}
            onChange={(e) => setHref(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                apply();
              } else if (e.key === "Escape") setLinking(false);
            }}
            placeholder="링크 주소"
          />
          <Btn label="적용" title="링크 적용 (Enter)" onClick={apply} wide />
          {s.link && <Btn label="링크 빼기" title="링크 지우기" onClick={() => (c().extendMarkRange("link").unsetLink().run(), setLinking(false))} wide />}
        </span>
      ) : (
        <>
          <Btn label="B" title="굵게 (Ctrl+B)" active={s.bold} onClick={() => c().toggleBold().run()} />
          <Btn label="I" title="기울임꼴 (Ctrl+I)" active={s.italic} onClick={() => c().toggleItalic().run()} />
          <Btn label="U" title="밑줄 (Ctrl+U)" active={s.underline} onClick={() => c().toggleUnderline().run()} />
          <Btn label="S" title="취소선" active={s.strike} onClick={() => c().toggleStrike().run()} />
          <Btn label="</>" title="코드" active={s.code} onClick={() => c().toggleCode().run()} />
          <span className="tb-sep" />
          <Btn label="🔗" title="링크 (Ctrl+K)" active={s.link} onClick={() => setLinking(true)} />
          {STATUS.map((x) => (
            <button
              key={x.color}
              type="button"
              className="status-pick"
              data-status={x.color}
              title={`상태 라벨: ${x.label}`}
              onMouseDown={keep}
              onClick={() => c().setMark("status", { color: x.color }).run()}
            />
          ))}
          {s.status && <Btn label="라벨 빼기" title="상태 라벨 지우기" onClick={() => c().unsetMark("status").run()} wide />}
          <Btn label="⌫" title="서식 지우기" onClick={() => c().unsetAllMarks().run()} />
        </>
      )}
    </BubbleMenu>
  );
}

/** Element of the block around the cursor, for placing a contextual bubble under it. */
function around(editor: Editor, selector: string) {
  const { node } = editor.view.domAtPos(editor.state.selection.from);
  const el = (node instanceof Element ? node : node.parentElement)?.closest(selector);
  return el ? { getBoundingClientRect: () => el.getBoundingClientRect() } : null;
}

/** Table tools under the table the cursor is in. */
export function TableBubble({ editor, onJoinBelow }: { editor: Editor; onJoinBelow: () => void }) {
  const below = useEditorState({ editor, selector: ({ editor: e }) => !!pairBelowCursor(e) });
  const c = () => editor.chain().focus();
  return (
    <BubbleMenu
      editor={editor}
      pluginKey="table"
      shouldShow={({ editor: e }) => e.isEditable && e.isActive("table")}
      getReferencedVirtualElement={() => around(editor, "table")}
      options={{ placement: "bottom-start", offset: 6 }}
      className="bubble"
    >
      <Btn label="↑ 행" title="위에 행 넣기" onClick={() => c().addRowBefore().run()} wide />
      <Btn label="↓ 행" title="아래에 행 넣기" onClick={() => c().addRowAfter().run()} wide />
      <Btn label="← 열" title="왼쪽에 열 넣기" onClick={() => c().addColumnBefore().run()} wide />
      <Btn label="→ 열" title="오른쪽에 열 넣기" onClick={() => c().addColumnAfter().run()} wide />
      <span className="tb-sep" />
      <Btn label="행 삭제" title="이 행 삭제" onClick={() => c().deleteRow().run()} wide />
      <Btn label="열 삭제" title="이 열 삭제" onClick={() => c().deleteColumn().run()} wide />
      <span className="tb-sep" />
      <Btn label="머리글 행" title="첫 행을 머리글로 켜기·끄기" onClick={() => c().toggleHeaderRow().run()} wide />
      <Btn label="셀 병합" title="선택한 셀 합치기 (셀을 끌어서 선택)" onClick={() => c().mergeCells().run()} wide />
      <Btn label="셀 나누기" title="합친 셀 나누기" onClick={() => c().splitCell().run()} wide />
      <span className="tb-sep" />
      {below && <Btn label="아래 표와 합치기" title="바로 아래 표를 이 표에 이어 붙입니다 (같은 머리글은 한 번만)" onClick={onJoinBelow} wide />}
      <Btn label="표 삭제" title="표 전체 삭제" onClick={() => c().deleteTable().run()} wide />
    </BubbleMenu>
  );
}

/** Panel type switcher under the panel the cursor is in. */
export function PanelBubble({ editor }: { editor: Editor }) {
  const type = useEditorState({ editor, selector: ({ editor: e }) => (e.getAttributes("panel").type as string | undefined) ?? "" });
  return (
    <BubbleMenu
      editor={editor}
      pluginKey="panel"
      shouldShow={({ editor: e }) => e.isEditable && e.isActive("panel") && !e.isActive("table")}
      getReferencedVirtualElement={() => around(editor, ".panel")}
      options={{ placement: "bottom-start", offset: 6 }}
      className="bubble"
    >
      {PANELS.map((p) => (
        <Btn
          key={p.type}
          label={`${p.icon} ${p.label}`}
          title={`${p.label} 패널로 바꾸기`}
          active={type === p.type}
          onClick={() => editor.chain().focus().updateAttributes("panel", { type: p.type }).run()}
          wide
        />
      ))}
      <span className="tb-sep" />
      <Btn label="패널 없애기" title="안의 글은 남기고 패널만 없앱니다" onClick={() => editor.chain().focus().lift("panel").run()} wide />
    </BubbleMenu>
  );
}
