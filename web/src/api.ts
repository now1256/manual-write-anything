// Types and transport for `mwa serve`. Mirrors crates/server/src/protocol.rs and
// the product map (crates/productmap/src/map.rs).

export type Who = "user" | "assistant" | "system" | "error";

export interface Quota {
  window: string;
  status: string;
  utilization: number | null;
  resets_at: number | null;
}

export interface Status {
  model: string;
  spent_usd: number;
  budget_usd: number;
  quota: Quota | null;
}

export interface SourceBadge {
  name: string;
  sha: string;
  preview: boolean;
}

export interface Header {
  workspace: string;
  sources: SourceBadge[];
}

export interface AppState {
  header: Header;
  status: Status;
  has_map: boolean;
  chat_models: string[];
  version: string;
}

export interface ApiLink {
  method: string;
  path: string;
  client: string;
  proxy: string | null;
  backend: string | null;
  operation: string | null;
  operation_id: string | null;
}

export interface Screen {
  route: string;
  file: string;
  kind: "menu" | "menu_detail" | "outside";
  reached_files: number;
  ui_permissions: string[];
  apis: ApiLink[];
}

export interface Feature {
  key: string;
  section: string;
  group: string | null;
  label: string;
  href: string;
  permissions: string[];
  license_menu: string;
  nav_line: number;
  screens: Screen[];
}

export interface ServerSite {
  file: string;
  line: number;
  route: string | null;
  operation: string | null;
}

export interface Permission {
  code: string;
  label: string;
  license_menu: string;
  licensable: boolean;
  server: ServerSite[];
  menus: string[];
  ui_pages: string[];
}

export interface Coverage {
  menus: number;
  menus_with_screen: number;
  screens: number;
  screens_with_api: number;
  client_calls: number;
  client_calls_linked: number;
  proxy_handlers: number;
  proxy_resolved: number;
  upstream_calls: number;
  upstream_with_operation: number;
}

export interface ProductMap {
  sources: {
    frontend_sha: string | null;
    backend_sha: string | null;
    openapi: { origin: string; sha256: string; operations: number } | null;
  };
  features: Feature[];
  other_screens: Screen[];
  permissions: Permission[];
  gaps: {
    unresolved_client_calls: unknown[];
    unlinked_client_calls: unknown[];
    unresolved_routes: string[];
    missing_backend_operations: string[];
    unknown_permission_codes: string[];
  };
  coverage: Coverage;
}

export interface ShotRecord {
  key: string;
  label: string;
  route: string;
  file: string | null;
  ok: boolean;
  error: string | null;
  width: number | null;
  height: number | null;
  sha256: string | null;
  blocked: { method: string; url: string }[];
  console_errors: string[];
  stable_diff: number | null;
  diff_box: [number, number, number, number] | null;
  unstable_file: string | null;
  run_errors: string[];
  toasts: string[];
  needs_retake: string | null;
  idle_timed_out: boolean;
  ms: number;
}

export interface ShotsManifest {
  taken_at: number;
  base_url: string;
  domain: string | null;
  frontend_sha: string | null;
  runs: number;
  viewport: [number, number];
  scale: number;
  shots: ShotRecord[];
}

export interface ManualFact {
  id: string;
  statement: string;
  kind: string;
  basis: string;
  source: { repo: string; file: string; line_start: number; line_end: number; quote: string } | null;
}

export interface ManualBlock {
  id: string;
  kind: "heading" | "paragraph" | "table" | "steps" | "callout" | "image" | "todo";
  markdown: string;
  facts: string[];
  shot: string | null;
}

export interface BlockCheck {
  id: string;
  issues: string[];
  verdict: string;
  reason: string;
}

export interface CompareItem {
  topic: string;
  generated: string | null;
  existing: string | null;
  status: "same" | "only_existing" | "only_generated" | "conflict";
  code_says: string | null;
  code_supports: string | null;
}

export interface ManualRun {
  feature: string;
  title: string;
  generated_at: number;
  models: string[];
  cost_usd: number;
  facts: ManualFact[];
  dropped_facts: number;
  blocks: ManualBlock[];
  checks: BlockCheck[];
  compare: CompareItem[] | null;
  compare_source: string | null;
}

export type ServerMsg =
  | { type: "delta"; id: number; text: string }
  | { type: "chat_done"; id: number }
  | { type: "lines"; id: number; title: string; lines: string[] }
  | { type: "map_built"; id: number; lines: string[] }
  | { type: "error"; id: number; message: string }
  | { type: "progress"; id: number; text: string }
  | { type: "shots_done"; id: number; lines: string[] }
  | { type: "manual_done"; id: number; key: string; lines: string[] }
  | { type: "status"; status: Status };

export type ClientMsg =
  | { type: "chat"; id: number; transcript: { who: Who; text: string }[]; selected: string | null }
  | { type: "command"; id: number; name: string; arg?: string };

async function getJson<T>(path: string): Promise<T | null> {
  const res = await fetch(path);
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`${path}: HTTP ${res.status}`);
  return (await res.json()) as T;
}

export const fetchState = () => getJson<AppState>("/api/state");
export const fetchMap = () => getJson<ProductMap>("/api/map");
export const fetchShots = () => getJson<ShotsManifest>("/api/shots");
export const fetchManual = (key: string) => getJson<ManualRun>(`/api/manual/${encodeURIComponent(key)}`);

/** URL of a captured file (`shots/menus/x.png` → served path), cache-busted by content hash. */
export function shotUrl(file: string, sha: string | null): string {
  const rel = file.replace(/^shots\//, "");
  return `/api/shots/file/${rel}${sha ? `?v=${sha.slice(0, 12)}` : ""}`;
}

/** Switches the chat model for this server session; returns the new status. */
export async function setChatModel(model: string): Promise<Status> {
  const res = await fetch("/api/chat-model", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ model }),
  });
  const body = (await res.json()) as { status?: Status; error?: string };
  if (!res.ok || !body.status) throw new Error(body.error ?? `HTTP ${res.status}`);
  return body.status;
}

/** WebSocket that reconnects on its own and reports connection changes. */
export class Channel {
  private ws: WebSocket | null = null;
  private closed = false;

  constructor(
    private onMessage: (m: ServerMsg) => void,
    private onConnected: (up: boolean) => void,
  ) {
    this.open();
  }

  private open() {
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    const ws = new WebSocket(`${scheme}://${location.host}/api/ws`);
    ws.onopen = () => this.onConnected(true);
    ws.onmessage = (e) => this.onMessage(JSON.parse(e.data as string) as ServerMsg);
    ws.onclose = () => {
      this.onConnected(false);
      if (!this.closed) setTimeout(() => this.open(), 1500);
    };
    this.ws = ws;
  }

  send(m: ClientMsg): boolean {
    if (this.ws?.readyState !== WebSocket.OPEN) return false;
    this.ws.send(JSON.stringify(m));
    return true;
  }

  close() {
    this.closed = true;
    this.ws?.close();
  }
}
