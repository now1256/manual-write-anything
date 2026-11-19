import type { Editor } from "@tiptap/react";
import type { Node as PMNode } from "@tiptap/pm/model";

/** Columns of a table row, counting merged cells. */
const columns = (row: PMNode | null) => {
  let n = 0;
  row?.forEach((cell) => (n += (cell.attrs.colspan as number) || 1));
  return n;
};

const isHeaderRow = (row: PMNode | null) => !!row && row.childCount > 0 && Array.from({ length: row.childCount }, (_, i) => row.child(i)).every((c) => c.type.name === "tableHeader");

const flat = (t: string) => t.replace(/\s+/g, "");

export interface SplitPair {
  /** Position of the first table; the second follows it directly. */
  from: number;
  to: number;
  first: PMNode;
  second: PMNode;
  /** The second table starts with the same header row: dropped when joined. */
  repeatsHeader: boolean;
}

/** Two tables next to each other with the same columns, the second without a different header. */
function pairAt(first: PMNode, second: PMNode, from: number, loose: boolean): SplitPair | null {
  if (first.type.name !== "table" || second.type.name !== "table") return null;
  if (columns(first.firstChild) !== columns(second.firstChild)) return null;
  const head = second.firstChild;
  const repeatsHeader = isHeaderRow(head) && flat(head!.textContent) === flat(first.firstChild?.textContent ?? "");
  if (isHeaderRow(head) && !repeatsHeader && !loose) return null;
  return { from, to: from + first.nodeSize + second.nodeSize, first, second, repeatsHeader };
}

/** Tables cut in two (by a page break when the PDF was converted). */
export function splitTables(editor: Editor): SplitPair[] {
  const tops: { node: PMNode; at: number }[] = [];
  editor.state.doc.forEach((node, at) => tops.push({ node, at }));
  const out: SplitPair[] = [];
  for (let i = 0; i + 1 < tops.length; i++) {
    const p = pairAt(tops[i].node, tops[i + 1].node, tops[i].at, false);
    if (p) out.push(p);
  }
  return out;
}

/** The pair made of the table the cursor is in and the one right below it, if any. */
export function pairBelowCursor(editor: Editor): SplitPair | null {
  const { $from } = editor.state.selection;
  if ($from.depth < 1) return null;
  const index = $from.index(0);
  const doc = editor.state.doc;
  if (index + 1 >= doc.childCount) return null;
  let at = 0;
  for (let i = 0; i < index; i++) at += doc.child(i).nodeSize;
  return pairAt(doc.child(index), doc.child(index + 1), at, true);
}

/** Joins pairs (bottom-up so positions stay valid); returns the joined pairs. */
export function joinTables(editor: Editor, pairs: SplitPair[]): SplitPair[] {
  const { tr, schema } = editor.state;
  const done: SplitPair[] = [];
  for (const p of [...pairs].sort((a, b) => b.from - a.from)) {
    const rows: PMNode[] = [];
    p.first.forEach((r) => rows.push(r));
    p.second.forEach((r, _o, i) => {
      if (!(i === 0 && p.repeatsHeader)) rows.push(r);
    });
    const joined = schema.nodes.table.create(p.first.attrs, rows);
    tr.replaceWith(p.from, p.to, joined);
    done.push(p);
  }
  if (done.length) editor.view.dispatch(tr);
  return done;
}
