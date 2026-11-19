import { useEffect, useRef, useState } from "react";
import type { Editor } from "@tiptap/react";

/** Requests offered on right-click; they open in the side panel. */
const PRESETS = ["맞춤법·문장만 다듬어 줘", "처음 쓰는 사람도 알기 쉽게 고쳐 줘", "뜻은 그대로 두고 짧게 줄여 줘", "표로 정리해 줘"];

/**
 * Right-click in the manual: a shortcut into the side panel (ask, preset requests, basis),
 * plus cut and copy. Shift+right-click keeps the browser's own menu.
 */
export function useContextMenu(
  editor: Editor | null,
  actions: { ask: (text?: string) => void; basis: () => void },
) {
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!at) return;
    const off = (e: MouseEvent) => !box.current?.contains(e.target as Node) && setAt(null);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setAt(null);
    document.addEventListener("mousedown", off);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", off);
      document.removeEventListener("keydown", esc);
    };
  }, [at]);

  const onContextMenu = (e: React.MouseEvent) => {
    if (!editor || e.shiftKey) return;
    e.preventDefault();
    if (editor.state.selection.empty) {
      // No selection: put the cursor where the click was, so the request is about that block.
      const pos = editor.view.posAtCoords({ left: e.clientX, top: e.clientY })?.pos;
      if (pos != null) editor.chain().setTextSelection(pos).run();
    }
    setAt({ x: Math.min(e.clientX, window.innerWidth - 280), y: Math.min(e.clientY, window.innerHeight - 330) });
  };

  const pick = (f: () => void) => () => {
    setAt(null);
    f();
  };
  const clip = (cut: boolean) =>
    pick(() => {
      editor?.commands.focus();
      document.execCommand(cut ? "cut" : "copy");
    });

  const menu = at && (
    <div className="ctx-menu" ref={box} style={{ left: at.x, top: at.y }}>
      <button className="mi" onClick={pick(() => actions.ask())}>
        <span className="mi-icon">✦</span> 도우미에게 요청…
      </button>
      {PRESETS.map((p) => (
        <button key={p} className="mi" onClick={pick(() => actions.ask(p))}>
          <span className="mi-icon">✎</span> {p}
        </button>
      ))}
      <button className="mi" onClick={pick(actions.basis)}>
        <span className="mi-icon">ⓘ</span> 이 부분의 근거 보기
      </button>
      <div className="ctx-sep" />
      <button className="mi" onClick={clip(true)}>잘라내기</button>
      <button className="mi" onClick={clip(false)}>복사</button>
      <div className="mi-hint">붙여넣기는 Ctrl+V · 브라우저 메뉴는 Shift+우클릭</div>
    </div>
  );

  return { onContextMenu, menu };
}
