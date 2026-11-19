import { Extension } from "@tiptap/react";
import { Plugin } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

/** Block types that can sit at the top level of a manual; each carries a `data-bid`. */
const TYPES = ["paragraph", "heading", "table", "bulletList", "orderedList", "taskList", "blockquote", "codeBlock", "horizontalRule", "panel", "figure", "details"];

export const newBid = () => `n${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

/**
 * Stable ids for top-level blocks, so the manual can say where each one came from (D-037).
 * Blocks without one, or a copy of one (split, paste), get a fresh id.
 */
export const BlockIds = Extension.create({
  name: "blockIds",
  addGlobalAttributes() {
    return [
      {
        types: TYPES,
        attributes: {
          bid: {
            default: null,
            keepOnSplit: false,
            parseHTML: (el) => el.getAttribute("data-bid"),
            renderHTML: (a) => (a.bid ? { "data-bid": a.bid } : {}),
          },
        },
      },
    ];
  },
  addProseMirrorPlugins() {
    return [
      new Plugin({
        appendTransaction: (trs, _old, state) => {
          if (!trs.some((t) => t.docChanged)) return null;
          const seen = new Set<string>();
          let tr = null as ReturnType<typeof state.tr.setNodeMarkup> | null;
          state.doc.forEach((node, offset) => {
            if (!TYPES.includes(node.type.name)) return;
            const bid = node.attrs.bid as string | null;
            if (bid && !seen.has(bid)) return void seen.add(bid);
            const fresh = newBid();
            seen.add(fresh);
            tr = (tr ?? state.tr).setNodeMarkup(offset, undefined, { ...node.attrs, bid: fresh });
          });
          return tr;
        },
      }),
    ];
  },
});

/** Gives every top-level element of an HTML fragment a `data-bid`; returns the HTML and the ids. */
export function withBids(html: string): { html: string; bids: string[] } {
  const body = new DOMParser().parseFromString(html, "text/html").body;
  const bids: string[] = [];
  for (const el of Array.from(body.children)) {
    let bid = el.getAttribute("data-bid");
    if (!bid) {
      bid = newBid();
      el.setAttribute("data-bid", bid);
    }
    bids.push(bid);
  }
  return { html: body.innerHTML, bids };
}

/** Top-level `data-bid` → text of an HTML fragment. */
export function bidTexts(html: string): Map<string, string> {
  const body = new DOMParser().parseFromString(html, "text/html").body;
  const out = new Map<string, string>();
  for (const el of Array.from(body.children)) {
    const bid = el.getAttribute("data-bid");
    if (bid) out.set(bid, (el.textContent ?? "").replace(/\s+/g, " ").trim());
  }
  return out;
}

/**
 * Shows a bulleted list whose items are all headings of the page like Confluence's table of
 * contents (small blue links); publishing turns it into the TOC macro.
 */
export const OutlineLook = Extension.create({
  name: "outlineLook",
  addProseMirrorPlugins() {
    return [
      new Plugin({
        props: {
          decorations: (state) => {
            const flat = (t: string) => t.replace(/\s+/g, "");
            const headings = new Set<string>();
            state.doc.forEach((n) => {
              if (n.type.name === "heading") headings.add(flat(n.textContent));
            });
            const marks: Decoration[] = [];
            state.doc.forEach((n, offset) => {
              if (n.type.name !== "bulletList") return;
              const items: string[] = [];
              n.descendants((d) => {
                if (d.type.name === "listItem") items.push(flat(d.firstChild?.textContent ?? ""));
              });
              // Almost every entry is a heading (an outline may name a step that is plain text).
              if (items.length >= 3 && items.filter((t) => t && headings.has(t)).length >= items.length * 0.8) {
                marks.push(Decoration.node(offset, offset + n.nodeSize, { class: "toc-list", title: "목차 (컨플루언스 게시 때 목차 매크로가 됩니다)" }));
              }
            });
            return DecorationSet.create(state.doc, marks);
          },
        },
      }),
    ];
  },
});
