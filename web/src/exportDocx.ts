import { forReaders } from "./confluenceStorage";
import {
  AlignmentType,
  BorderStyle,
  Document,
  ExternalHyperlink,
  HeadingLevel,
  ImageRun,
  LevelFormat,
  Packer,
  Paragraph,
  ShadingType,
  Table,
  TableCell,
  TableRow,
  TextRun,
  WidthType,
  type ParagraphChild,
} from "docx";

/**
 * The manual as a Word file for Confluence Cloud's import (More actions → Templates and import →
 * Import → .docx), which turns headings, lists, tables and pictures into a native page.
 */

/** Usable width of a page body in pixels (A4 with normal margins). */
const PAGE_PX = 600;

const PANEL: Record<string, { fill: string; label: string }> = {
  info: { fill: "E9F2FF", label: "ℹ 정보" },
  note: { fill: "F3F0FF", label: "✎ 참고" },
  warning: { fill: "FFF7D6", label: "⚠ 주의" },
  success: { fill: "DCFFF1", label: "✓ 성공" },
};

const STATUS: Record<string, { color: string; fill: string }> = {
  grey: { color: "44546F", fill: "DCDFE4" },
  blue: { color: "0055CC", fill: "CCE0FF" },
  green: { color: "216E4E", fill: "BAF3DB" },
  yellow: { color: "7F5F01", fill: "F8E6A0" },
  red: { color: "AE2E24", fill: "FFD5D2" },
};

interface Style {
  bold?: boolean;
  italics?: boolean;
  underline?: boolean;
  strike?: boolean;
  code?: boolean;
  color?: string;
  status?: string;
}

const hex = (css: string | null) => {
  if (!css) return undefined;
  const m = css.match(/#([0-9a-f]{6})/i);
  if (m) return m[1];
  const rgb = css.match(/rgb\((\d+),\s*(\d+),\s*(\d+)\)/);
  return rgb ? rgb.slice(1).map((n) => Number(n).toString(16).padStart(2, "0")).join("") : undefined;
};

function runs(node: Node, style: Style = {}): ParagraphChild[] {
  const out: ParagraphChild[] = [];
  node.childNodes.forEach((n) => {
    if (n.nodeType === Node.TEXT_NODE) {
      const text = n.textContent ?? "";
      if (!text) return;
      const st = style.status ? STATUS[style.status] ?? STATUS.grey : null;
      out.push(
        new TextRun({
          text: st ? ` ${text.toUpperCase()} ` : text,
          bold: style.bold || !!st,
          italics: style.italics,
          underline: style.underline ? {} : undefined,
          strike: style.strike,
          font: style.code ? "Consolas" : undefined,
          color: st?.color ?? style.color,
          size: st ? 18 : undefined,
          shading: st ? { type: ShadingType.CLEAR, fill: st.fill, color: "auto" } : style.code ? { type: ShadingType.CLEAR, fill: "F1F2F4", color: "auto" } : undefined,
        }),
      );
      return;
    }
    if (!(n instanceof HTMLElement)) return;
    const tag = n.tagName;
    if (tag === "BR") return void out.push(new TextRun({ text: "", break: 1 }));
    const next: Style = { ...style };
    if (tag === "STRONG" || tag === "B") next.bold = true;
    if (tag === "EM" || tag === "I") next.italics = true;
    if (tag === "U") next.underline = true;
    if (tag === "S" || tag === "DEL") next.strike = true;
    if (tag === "CODE") next.code = true;
    if (n.dataset.status) next.status = n.dataset.status;
    const color = hex(n.style.color);
    if (color) next.color = color;
    if (tag === "A" && n.getAttribute("href")) {
      out.push(new ExternalHyperlink({ link: n.getAttribute("href")!, children: runs(n, { ...next, underline: true, color: "0C66E4" }) as TextRun[] }));
      return;
    }
    out.push(...runs(n, next));
  });
  return out;
}

const align = (el: HTMLElement) =>
  ({ center: AlignmentType.CENTER, right: AlignmentType.RIGHT })[el.style.textAlign] ?? undefined;

/** Turns `src` into image bytes and a page-fitting size. */
async function picture(src: string, widthPct: number | null): Promise<ImageRun | null> {
  try {
    const res = await fetch(src);
    if (!res.ok) return null;
    const blob = await res.blob();
    const bmp = await createImageBitmap(blob);
    const ratio = bmp.height / bmp.width;
    const want = widthPct ? (PAGE_PX * widthPct) / 100 : Math.min(PAGE_PX, bmp.width);
    const type = blob.type.includes("jpeg") ? "jpg" : blob.type.includes("gif") ? "gif" : "png";
    return new ImageRun({ type, data: await blob.arrayBuffer(), transformation: { width: Math.round(want), height: Math.round(want * ratio) } });
  } catch {
    return null;
  }
}

interface Ctx {
  /** Image URL of a figure as shown in the editor (current screen, upload, old picture). */
  srcOf: (fig: HTMLElement) => string | null;
  /** Distinct ordered lists restart at 1. */
  lists: number;
}

async function blocks(el: Element, ctx: Ctx, level = 0): Promise<(Paragraph | Table)[]> {
  const out: (Paragraph | Table)[] = [];
  for (const child of Array.from(el.children) as HTMLElement[]) {
    const tag = child.tagName;
    if (/^H[1-6]$/.test(tag)) {
      const heading = [HeadingLevel.HEADING_1, HeadingLevel.HEADING_2, HeadingLevel.HEADING_3][Math.min(Number(tag[1]) - 1, 2)];
      out.push(new Paragraph({ heading, children: runs(child), alignment: align(child) }));
    } else if (tag === "P") {
      out.push(new Paragraph({ children: runs(child), alignment: align(child) }));
    } else if (tag === "UL" || tag === "OL") {
      const task = child.dataset.type === "taskList";
      const instance = tag === "OL" ? ++ctx.lists : 0;
      for (const li of Array.from(child.children) as HTMLElement[]) {
        const body = li.querySelector(":scope > div") ?? li;
        const nested = Array.from(body.children).filter((c) => c.tagName === "UL" || c.tagName === "OL");
        const own = Array.from(body.children).filter((c) => !nested.includes(c));
        const text: ParagraphChild[] = task ? [new TextRun(li.dataset.checked === "true" ? "☑ " : "☐ ")] : [];
        own.forEach((c, i) => {
          if (i > 0) text.push(new TextRun({ text: "", break: 1 }));
          text.push(...runs(c));
        });
        if (!own.length) text.push(...runs(body));
        out.push(
          new Paragraph({
            children: text,
            numbering: task ? undefined : { reference: tag === "OL" ? "numbers" : "bullets", level: Math.min(level, 4), instance },
            indent: task ? { left: 360 * (level + 1) } : undefined,
          }),
        );
        for (const n of nested) {
          const wrap = document.createElement("div");
          wrap.appendChild(n.cloneNode(true));
          out.push(...(await blocks(wrap, ctx, level + 1)));
        }
      }
    } else if (tag === "TABLE" || child.classList.contains("tableWrapper")) {
      const table = tag === "TABLE" ? child : child.querySelector("table");
      if (!table) continue;
      const rows: TableRow[] = [];
      for (const tr of Array.from(table.querySelectorAll("tr"))) {
        const cells: TableCell[] = [];
        for (const td of Array.from(tr.children) as HTMLElement[]) {
          const head = td.tagName === "TH";
          const inner = td.querySelector("p, ul, ol") ? await blocks(td, ctx) : [new Paragraph({ children: runs(td) })];
          cells.push(
            new TableCell({
              children: inner.length ? inner : [new Paragraph("")],
              columnSpan: Number(td.getAttribute("colspan")) || 1,
              rowSpan: Number(td.getAttribute("rowspan")) || 1,
              shading: head ? { type: ShadingType.CLEAR, fill: "F1F2F4", color: "auto" } : undefined,
            }),
          );
        }
        rows.push(new TableRow({ children: cells, tableHeader: tr.querySelector("th") != null && rows.length === 0 }));
      }
      if (rows.length) out.push(new Table({ rows, width: { size: 100, type: WidthType.PERCENTAGE } }));
      out.push(new Paragraph(""));
    } else if (child.dataset.panel) {
      const p = PANEL[child.dataset.panel] ?? PANEL.info;
      const inner = await blocks(child, ctx);
      out.push(
        new Table({
          width: { size: 100, type: WidthType.PERCENTAGE },
          rows: [
            new TableRow({
              children: [
                new TableCell({
                  shading: { type: ShadingType.CLEAR, fill: p.fill, color: "auto" },
                  borders: {
                    top: { style: BorderStyle.NONE, size: 0, color: "FFFFFF" },
                    bottom: { style: BorderStyle.NONE, size: 0, color: "FFFFFF" },
                    left: { style: BorderStyle.SINGLE, size: 18, color: "0C66E4" },
                    right: { style: BorderStyle.NONE, size: 0, color: "FFFFFF" },
                  },
                  children: [new Paragraph({ children: [new TextRun({ text: p.label, bold: true })] }), ...inner],
                }),
              ],
            }),
          ],
        }),
      );
      out.push(new Paragraph(""));
    } else if (tag === "FIGURE") {
      // Pictures side by side (one row) go into one line; the first of the row writes them all.
      const row = child.dataset.row;
      const prev = child.previousElementSibling as HTMLElement | null;
      if (row && prev?.tagName === "FIGURE" && prev.dataset.row === row) continue;
      const group = [child];
      for (let n = child.nextElementSibling as HTMLElement | null; row && n?.tagName === "FIGURE" && n.dataset.row === row; n = n.nextElementSibling as HTMLElement | null) group.push(n);
      const imgs: (ImageRun | TextRun)[] = [];
      for (const f of group) {
        const src = ctx.srcOf(f);
        const img = src ? await picture(src, Number(f.dataset.width) || null) : null;
        if (img) imgs.push(...(imgs.length ? [new TextRun("  ")] : []), img);
      }
      if (imgs.length) out.push(new Paragraph({ children: imgs, alignment: AlignmentType.CENTER }));
      const caption = group
        .map((f) => f.querySelector("figcaption")?.textContent?.trim())
        .filter(Boolean)
        .join(" / ");
      if (caption) out.push(new Paragraph({ children: [new TextRun({ text: caption, italics: true, color: "626F86" })], alignment: AlignmentType.CENTER }));
    } else if (tag === "BLOCKQUOTE") {
      for (const p of await blocks(child, ctx)) out.push(p);
    } else if (tag === "PRE") {
      for (const line of (child.textContent ?? "").split("\n")) {
        out.push(new Paragraph({ children: [new TextRun({ text: line, font: "Consolas", size: 18 })], shading: { type: ShadingType.CLEAR, fill: "F7F8F9", color: "auto" } }));
      }
    } else if (tag === "HR") {
      out.push(new Paragraph({ border: { bottom: { style: BorderStyle.SINGLE, size: 6, color: "DCDFE4", space: 1 } } }));
    } else if (tag === "DIV" && child.dataset.type === "details") {
      out.push(...(await blocks(child, ctx)));
    } else if (tag === "SUMMARY") {
      out.push(new Paragraph({ children: runs(child, { bold: true }) }));
    } else if (child.children.length) {
      out.push(...(await blocks(child, ctx, level)));
    } else if (child.textContent?.trim()) {
      out.push(new Paragraph({ children: runs(child) }));
    }
  }
  return out;
}

const bulletLevels = Array.from({ length: 5 }, (_, i) => ({
  level: i,
  format: LevelFormat.BULLET,
  text: ["•", "◦", "▪", "•", "◦"][i],
  alignment: AlignmentType.LEFT,
  style: { paragraph: { indent: { left: 360 * (i + 1), hanging: 260 } } },
}));

const numberLevels = Array.from({ length: 5 }, (_, i) => ({
  level: i,
  format: [LevelFormat.DECIMAL, LevelFormat.LOWER_LETTER, LevelFormat.LOWER_ROMAN, LevelFormat.DECIMAL, LevelFormat.LOWER_LETTER][i],
  text: `%${i + 1}.`,
  alignment: AlignmentType.LEFT,
  style: { paragraph: { indent: { left: 360 * (i + 1), hanging: 260 } } },
}));

/** Builds the Word file for a manual's HTML and starts the download. */
export async function exportDocx(title: string, html: string, srcOf: Ctx["srcOf"]): Promise<void> {
  const body = new DOMParser().parseFromString(forReaders(html), "text/html").body;
  const children = await blocks(body, { srcOf, lists: 0 });
  const doc = new Document({
    creator: "mwa",
    title,
    styles: { default: { document: { run: { font: "맑은 고딕", size: 21 } } } },
    numbering: {
      config: [
        { reference: "bullets", levels: bulletLevels },
        { reference: "numbers", levels: numberLevels },
      ],
    },
    sections: [{ children: [new Paragraph({ heading: HeadingLevel.TITLE, children: [new TextRun(title)] }), ...children] }],
  });
  const blob = await Packer.toBlob(doc);
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `${title}.docx`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 10000);
}
