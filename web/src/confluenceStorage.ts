/**
 * Editor HTML → Confluence storage format (XHTML with Confluence macros), so a published page
 * uses Confluence's own panels, status lozenges, expands and attached pictures (D-041).
 */

const esc = (t: string) => t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/**
 * Panels → Confluence macros: info (blue), note → a purple panel like Confluence's note panel,
 * warning → note macro (yellow), success → tip (green).
 */
const PANEL: Record<string, string> = { info: "info", warning: "note", success: "tip" };
const NOTE_PANEL = (body: string) =>
  `<ac:structured-macro ac:name="panel"><ac:parameter ac:name="bgColor">#EAE6FF</ac:parameter><ac:parameter ac:name="borderStyle">none</ac:parameter><ac:rich-text-body>${body}</ac:rich-text-body></ac:structured-macro>`;

const STATUS: Record<string, string> = { grey: "Grey", blue: "Blue", green: "Green", yellow: "Yellow", red: "Red" };

/** Confluence page body width in pixels; pictures are sized against it. */
const PAGE_PX = 760;
/** Width of a picture with no size set (Confluence's usual inserted size). */
const DEFAULT_PX = 590;

export interface StorageImage {
  /** Attachment file name. */
  name: string;
  /** Where the editor shows it (fetched by the engine). */
  url: string;
}

interface Ctx {
  srcOf: (fig: HTMLElement) => string | null;
  images: StorageImage[];
  tasks: number;
  /** Heading texts of the document, to recognise an outline list (→ table of contents). */
  headings: Set<string>;
}

const flat = (t: string | null | undefined) => (t ?? "").replace(/\s+/g, "");

/** A bulleted list whose items are all headings of the page: Confluence's TOC macro does that. */
function isOutline(list: HTMLElement, ctx: Ctx): boolean {
  const items = Array.from(list.querySelectorAll("li")).map((li) => {
    const own = li.querySelector(":scope > p") ?? li;
    return flat(Array.from(own.childNodes).filter((n) => n.nodeType === Node.TEXT_NODE || (n as HTMLElement).tagName !== "UL").map((n) => n.textContent).join(""));
  });
  return items.length >= 3 && items.filter((t) => t && ctx.headings.has(t)).length >= items.length * 0.8;
}

const alignStyle = (el: HTMLElement) => (el.style.textAlign && el.style.textAlign !== "left" ? ` style="text-align: ${el.style.textAlign};"` : "");

function inline(node: Node): string {
  let out = "";
  node.childNodes.forEach((n) => {
    if (n.nodeType === Node.TEXT_NODE) return void (out += esc(n.textContent ?? ""));
    if (!(n instanceof HTMLElement)) return;
    const inner = () => inline(n);
    switch (n.tagName) {
      case "BR":
        out += "<br />";
        return;
      case "STRONG":
      case "B":
        out += `<strong>${inner()}</strong>`;
        return;
      case "EM":
      case "I":
        out += `<em>${inner()}</em>`;
        return;
      case "U":
        out += `<u>${inner()}</u>`;
        return;
      case "S":
      case "DEL":
        out += `<s>${inner()}</s>`;
        return;
      case "CODE":
        out += `<code>${inner()}</code>`;
        return;
      case "A": {
        const href = n.getAttribute("href");
        out += href ? `<a href="${esc(href)}">${inner()}</a>` : inner();
        return;
      }
    }
    if (n.dataset.status) {
      out += `<ac:structured-macro ac:name="status"><ac:parameter ac:name="colour">${STATUS[n.dataset.status] ?? "Grey"}</ac:parameter><ac:parameter ac:name="title">${esc(n.textContent ?? "")}</ac:parameter></ac:structured-macro>`;
      return;
    }
    if (n.style.color) {
      out += `<span style="color: ${esc(n.style.color)};">${inner()}</span>`;
      return;
    }
    out += inner();
  });
  return out;
}

/** Block children of a list item or cell: a single paragraph is written inline. */
function itemBody(el: HTMLElement, ctx: Ctx): string {
  const kids = Array.from(el.children) as HTMLElement[];
  if (kids.length === 1 && kids[0].tagName === "P") return inline(kids[0]);
  if (!kids.length) return inline(el);
  return blocks(el, ctx);
}

function blocks(el: Element, ctx: Ctx): string {
  let out = "";
  for (const b of Array.from(el.children) as HTMLElement[]) {
    const tag = b.tagName;
    if (/^H[1-6]$/.test(tag)) out += `<${tag.toLowerCase()}${alignStyle(b)}>${inline(b)}</${tag.toLowerCase()}>`;
    else if (tag === "P") out += `<p${alignStyle(b)}>${inline(b)}</p>`;
    else if (tag === "UL" && b.dataset.type === "taskList") {
      out += "<ac:task-list>";
      for (const li of Array.from(b.children) as HTMLElement[]) {
        const body = (li.querySelector(":scope > div") as HTMLElement | null) ?? li;
        out += `<ac:task><ac:task-id>${++ctx.tasks}</ac:task-id><ac:task-status>${li.dataset.checked === "true" ? "complete" : "incomplete"}</ac:task-status><ac:task-body>${itemBody(body, ctx)}</ac:task-body></ac:task>`;
      }
      out += "</ac:task-list>";
    } else if (tag === "UL" && isOutline(b, ctx)) {
      out += `<ac:structured-macro ac:name="toc"><ac:parameter ac:name="maxLevel">3</ac:parameter></ac:structured-macro>`;
    } else if (tag === "UL" || tag === "OL") {
      out += `<${tag.toLowerCase()}>`;
      for (const li of Array.from(b.children) as HTMLElement[]) out += `<li>${itemBody(li, ctx)}</li>`;
      out += `</${tag.toLowerCase()}>`;
    } else if (tag === "TABLE" || b.classList.contains("tableWrapper")) {
      const table = tag === "TABLE" ? b : b.querySelector("table");
      if (!table) continue;
      out += "<table><tbody>";
      for (const tr of Array.from(table.querySelectorAll("tr"))) {
        out += "<tr>";
        for (const td of Array.from(tr.children) as HTMLElement[]) {
          const t = td.tagName === "TH" ? "th" : "td";
          const span = `${td.getAttribute("colspan") && td.getAttribute("colspan") !== "1" ? ` colspan="${td.getAttribute("colspan")}"` : ""}${td.getAttribute("rowspan") && td.getAttribute("rowspan") !== "1" ? ` rowspan="${td.getAttribute("rowspan")}"` : ""}`;
          const kids = Array.from(td.children);
          out += `<${t}${span}>${kids.length ? blocks(td, ctx) : `<p>${inline(td)}</p>`}</${t}>`;
        }
        out += "</tr>";
      }
      out += "</tbody></table>";
    } else if (b.dataset.panel) {
      const body = blocks(b, ctx);
      out +=
        b.dataset.panel === "note"
          ? NOTE_PANEL(body)
          : `<ac:structured-macro ac:name="${PANEL[b.dataset.panel] ?? "info"}"><ac:rich-text-body>${body}</ac:rich-text-body></ac:structured-macro>`;
    } else if (tag === "FIGURE") {
      // Pictures side by side (one row) go inline into one paragraph; the first writes them all.
      const row = b.dataset.row;
      const prev = b.previousElementSibling as HTMLElement | null;
      if (row && prev?.tagName === "FIGURE" && prev.dataset.row === row) continue;
      const group = [b];
      for (let n = b.nextElementSibling as HTMLElement | null; row && n?.tagName === "FIGURE" && n.dataset.row === row; n = n.nextElementSibling as HTMLElement | null) group.push(n);
      const imgs: string[] = [];
      for (const f of group) {
        const src = ctx.srcOf(f);
        if (!src) continue;
        const ext = /\.jpe?g(\?|$)/i.test(src) ? "jpg" : "png";
        const name = `${f.dataset.bid ?? `pic-${ctx.images.length + 1}`}.${ext}`;
        ctx.images.push({ name, url: src });
        const pct = Number(f.dataset.width);
        const width = pct ? Math.round((PAGE_PX * pct) / 100) : DEFAULT_PX;
        imgs.push(
          group.length > 1
            ? `<ac:image ac:width="${width}"><ri:attachment ri:filename="${esc(name)}" /></ac:image>`
            : `<ac:image ac:align="center" ac:layout="center" ac:width="${width}"><ri:attachment ri:filename="${esc(name)}" /></ac:image>`,
        );
      }
      if (imgs.length > 1) out += `<p style="text-align: center;">${imgs.join(" ")}</p>`;
      else out += imgs.join("");
      const caption = group
        .map((f) => f.querySelector("figcaption")?.textContent?.trim())
        .filter(Boolean)
        .join(" / ");
      if (caption) out += `<p style="text-align: center;"><em>${esc(caption)}</em></p>`;
    } else if (tag === "BLOCKQUOTE") out += `<blockquote>${blocks(b, ctx)}</blockquote>`;
    else if (tag === "PRE") {
      const code = (b.textContent ?? "").replace(/]]>/g, "]]]]><![CDATA[>");
      out += `<ac:structured-macro ac:name="code"><ac:plain-text-body><![CDATA[${code}]]></ac:plain-text-body></ac:structured-macro>`;
    } else if (tag === "HR") out += "<hr />";
    else if (b.dataset.type === "details") {
      const summary = b.querySelector("summary")?.textContent?.trim() ?? "펼치기";
      const content = (b.querySelector('[data-type="detailsContent"]') as HTMLElement | null) ?? b;
      out += `<ac:structured-macro ac:name="expand"><ac:parameter ac:name="title">${esc(summary)}</ac:parameter><ac:rich-text-body>${blocks(content, ctx)}</ac:rich-text-body></ac:structured-macro>`;
    } else if (b.children.length) out += blocks(b, ctx);
    else if (b.textContent?.trim()) out += `<p>${inline(b)}</p>`;
  }
  return out;
}

/** Outline (table of contents) list items with their depth: 0 = top level. */
function outlineDepths(body: HTMLElement, headings: Set<string>): Map<string, number> {
  const out = new Map<string, number>();
  const list = Array.from(body.children).find((c) => c.tagName === "UL") as HTMLElement | undefined;
  if (!list) return out;
  const walk = (ul: Element, depth: number) => {
    for (const li of Array.from(ul.children)) {
      const own = Array.from(li.childNodes)
        .filter((n) => !(n instanceof HTMLElement && (n.tagName === "UL" || n.tagName === "OL")))
        .map((n) => n.textContent)
        .join("");
      if (headings.has(flat(own))) out.set(flat(own), depth);
      li.querySelectorAll(":scope > ul, :scope > ol").forEach((sub) => walk(sub, depth + 1));
    }
  };
  walk(list, 0);
  return out;
}

/**
 * House-format clean-up before publishing: the page title is not repeated as the first heading,
 * and heading levels follow the outline (top level → 제목 2, next → 제목 3, …), as in the old
 * Confluence pages.
 */
export function tidy(html: string, title: string): string {
  const body = new DOMParser().parseFromString(html, "text/html").body;
  const first = body.firstElementChild;
  if (first && /^H[1-6]$/.test(first.tagName) && flat(first.textContent) === flat(title)) first.remove();
  const headings = new Set(Array.from(body.querySelectorAll("h1, h2, h3, h4")).map((h) => flat(h.textContent)));
  const depth = outlineDepths(body, headings);
  if (depth.size >= 3) {
    for (const h of Array.from(body.querySelectorAll("h1, h2, h3, h4"))) {
      const d = depth.get(flat(h.textContent));
      if (d == null) continue;
      const level = `H${Math.min(2 + d, 4)}`;
      if (h.tagName === level) continue;
      const n = document.createElement(level);
      for (const a of Array.from(h.attributes)) n.setAttribute(a.name, a.value);
      n.innerHTML = h.innerHTML;
      h.replaceWith(n);
    }
  }
  return body.innerHTML;
}

/**
 * The manual as readers get it: "확인 필요" panels are notes for the editors, and picture slots
 * not shot yet have nothing to show; both stay behind.
 */
export function forReaders(html: string): string {
  const body = new DOMParser().parseFromString(html, "text/html").body;
  for (const p of Array.from(body.querySelectorAll(":scope > div[data-panel]")))
    if (flat(p.textContent).startsWith("확인필요")) p.remove();
  for (const f of Array.from(body.querySelectorAll(":scope > figure[data-want]")))
    if (!f.hasAttribute("data-shot") && !f.hasAttribute("data-asset")) f.remove();
  return body.innerHTML;
}

/** Storage-format body and the pictures it attaches. */
export function toStorage(html: string, srcOf: Ctx["srcOf"], title = ""): { storage: string; images: StorageImage[] } {
  const body = new DOMParser().parseFromString(forReaders(title ? tidy(html, title) : html), "text/html").body;
  const headings = new Set(Array.from(body.querySelectorAll("h1, h2, h3, h4")).map((h) => flat(h.textContent)));
  const ctx: Ctx = { srcOf, images: [], tasks: 0, headings };
  return { storage: blocks(body, ctx), images: ctx.images };
}
