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

export interface ProjectEntry {
  name: string;
  path: string;
  /** The folder still has a mwa.toml. */
  ready: boolean;
  last_opened: number | null;
}

export interface ProjectList {
  current: string;
  items: ProjectEntry[];
  switchable: boolean;
}

export const fetchProjects = () => getJson<ProjectList>("/api/projects");

export interface Settings {
  sources: { name: string; path: string; git: string; branch: string; ssh?: string; jump?: string; docker?: string; remote_path?: string }[];
  product: { name: string; adapter: string; frontend: string; backend: string; openapi: string } | null;
  profiles: { name: string; base_url: string; user_id: string; purpose: string }[];
  capture: { profile: string; domain: string; viewport: [number, number]; scale: number; node: string } | null;
  roles: { role: string; backend: string; model: string; cross_model?: string; uses: string }[];
  backends: string[];
  max_usd_per_run: number;
  template: string;
  agents: { backend: string; command: string }[];
  /** [name, path] of adapters in the user's adapter folder. */
  known_adapters: [string, string][];
  /** Add-on: publish to Confluence. */
  confluence: boolean;
}

export interface Check {
  ok: boolean;
  detail: string;
}

export interface SetupStep {
  key: string;
  label: string;
  done: boolean;
  detail: string;
  /** "settings", "command:<name>" or "". */
  action: string;
}

export interface FolderListing {
  path: string;
  parent: string | null;
  entries: { name: string; dir: boolean; git: boolean }[];
}

export interface RefSummary {
  id: string;
  title: string;
  tag: string | null;
  feature: string | null;
  ignored: boolean;
  pages: number;
  images: number;
  described: number;
  source: string;
  imported_at: number;
}

export interface RefList {
  items: RefSummary[];
  features: { key: string; label: string; group: string | null }[];
}

export interface Scene {
  kind: string;
  screen: string;
  state: string;
  capture: boolean;
  how: string;
}

export interface RefImage {
  file: string;
  page: number;
  width: number;
  height: number;
  heading: string;
  context: string;
  caption: string;
  scene?: Scene;
}

export interface RefDoc {
  id: string;
  title: string;
  tag: string | null;
  source: string;
  feature: string | null;
  pages: number;
  imported_at: number;
  images: RefImage[];
}

export interface SceneShot {
  image: string;
  page: number;
  heading: string;
  what: string;
  /** reproduced | different | failed */
  status: string;
  reason_kind?: string;
  reason?: string;
  differences: string[];
  /** What changed in the product's screen itself; the manual's text follows these. */
  changes?: string[];
  /** The capture plan that was used (replayed by the quick re-shoot). */
  plan?: unknown;
  /** A person's instruction for shooting this scene. */
  instruction?: string | null;
  file?: string;
  data?: string;
  /** First plan, then each re-plan. */
  attempts?: SceneAttempt[];
  /** Write requests stopped (no demo response). */
  blocked?: string[];
  /** Requests answered from demo data. */
  mocked?: string[];
  /** Picture a person uploaded for this scene (document asset file). */
  uploaded?: string;
  /** Picture chosen for the new manual: current | old | upload (none = automatic). */
  choice?: string;
}

export interface SceneAttempt {
  steps: { text: string; ok: boolean | null; ms: number; error?: string }[];
  error?: string;
  fail_file?: string;
}

export interface SceneShots {
  feature: string;
  doc: string;
  taken_at: number;
  scenes: SceneShot[];
}

export const fetchSceneShots = (feature: string) =>
  getJson<SceneShots>(`/api/references/shots/${encodeURIComponent(feature)}`);

export interface VersionMeta {
  n: number;
  created_at: number;
  sources: { name: string; sha: string; branch: string | null; preview: boolean }[];
  template: string;
  blocks: number;
  images: number;
  supported: number;
  partial: number;
  unsupported: number;
}

export interface VersionDiff {
  against: string;
  added: string[];
  removed: string[];
  changed: string[];
  images_added: number;
  images_removed: number;
}

export async function fetchVersions(feature: string): Promise<{ meta: VersionMeta; diff: VersionDiff }[]> {
  const r = await getJson<{ versions: { meta: VersionMeta; diff: VersionDiff }[] }>(
    `/api/manual/${encodeURIComponent(feature)}/versions`,
  );
  return r?.versions ?? [];
}

export const fetchVersion = (feature: string, n: number) =>
  getJson<ManualRun>(`/api/manual/${encodeURIComponent(feature)}/versions/${n}`);

export async function fetchReferences(): Promise<RefList> {
  return (await getJson<RefList>("/api/references")) ?? { items: [], features: [] };
}

export const fetchReference = (id: string) => getJson<RefDoc>(`/api/references/${encodeURIComponent(id)}`);

export const importReferences = (paths: string[]) => postJson<{ lines: string[] }>("/api/references/import", { paths });

export const setReference = (id: string, feature: string | null, ignored: boolean) =>
  postJson<{ ok: boolean }>(`/api/references/${encodeURIComponent(id)}`, { feature, ignored });

export const refFileUrl = (id: string, file: string) => `/api/references/${encodeURIComponent(id)}/file/${file}`;
export const refPageUrl = (id: string, page: number) => `/api/references/${encodeURIComponent(id)}/page/${page}`;

/** Read-only probe of one settings value (folder, git, adapter, openapi, site, file, command). */
export const checkField = (kind: string, value: string, extra = "") =>
  postJson<Check>("/api/check", { kind, value, extra });

export async function fetchSetup(): Promise<SetupStep[]> {
  const r = await getJson<{ steps: SetupStep[] }>("/api/setup");
  return r?.steps ?? [];
}

export async function browseFolder(path: string | null, ext = ""): Promise<FolderListing> {
  const q = new URLSearchParams();
  if (path) q.set("path", path);
  if (ext) q.set("ext", ext);
  const res = await fetch(`/api/fs?${q}`);
  const data = (await res.json()) as FolderListing & { error?: string };
  if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
  return data;
}

export async function fetchSettings(): Promise<Settings> {
  const s = await getJson<Settings>("/api/settings");
  if (!s) throw new Error("설정이 없습니다");
  return s;
}

/** Saves mwa.toml (validated by the engine) and reloads the project on the server. */
export async function saveSettings(s: Settings): Promise<{ note: string }> {
  const res = await fetch("/api/settings", {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(s),
  });
  const data = (await res.json().catch(() => ({}))) as { note?: string; error?: string };
  if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
  return { note: data.note ?? "저장했습니다" };
}

export const setPassword = (profile: string, password: string) =>
  postJson<{ note: string }>("/api/settings/password", { profile, password });

async function postJson<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const data = (await res.json().catch(() => ({}))) as T & { error?: string };
  if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
  return data;
}

/** Creates a project folder (default location when `path` is empty). */
export const createProject = (name: string, path?: string) =>
  postJson<ProjectEntry>("/api/projects", { name, path: path || null });

/** Switches the engine to another project; the page should reload afterwards. */
export const openProject = (path: string) => postJson<{ current: string }>("/api/projects/open", { path });

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

/** Editable manual document (D-032). */
export interface ManualDoc {
  feature: string;
  title: string;
  /** `pdf:<reference id>` or `generated`. */
  origin: string;
  reference: string | null;
  html: string;
  version: number;
  updated_at: number;
  /** The version that is the old manual as imported. */
  base?: number | null;
}

export const fetchDoc = (feature: string) => getJson<ManualDoc>(`/api/docs/${encodeURIComponent(feature)}`);

/** Why some blocks changed (accepted proposal, applied request), saved with the document. */
export interface DocChange {
  bids: string[];
  kind: string;
  summary: string;
  request?: string | null;
  facts: string[];
  inherit: string[];
}

/** `base`: the version the edits started from; the engine refuses if the document moved on. */
export async function saveDoc(feature: string, html: string, changes: DocChange[] = [], base?: number): Promise<ManualDoc> {
  const res = await fetch(`/api/docs/${encodeURIComponent(feature)}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ html, changes, base }),
  });
  const data = (await res.json().catch(() => ({}))) as ManualDoc & { error?: string };
  if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
  return data;
}

/** Uploads a picture for a document; returns the stored file name. */
export async function uploadDocAsset(feature: string, file: File): Promise<string> {
  const res = await fetch(`/api/docs/${encodeURIComponent(feature)}/assets?name=${encodeURIComponent(file.name)}`, {
    method: "POST",
    body: file,
  });
  const data = (await res.json().catch(() => ({}))) as { file?: string; error?: string };
  if (!res.ok || !data.file) throw new Error(data.error ?? `HTTP ${res.status}`);
  return data.file;
}

export const docAssetUrl = (feature: string, file: string) =>
  `/api/docs/${encodeURIComponent(feature)}/assets/${encodeURIComponent(file)}`;

export const fetchDocVersions = async (feature: string) =>
  (await getJson<{ version: number; updated_at: number }[]>(`/api/docs/${encodeURIComponent(feature)}/versions`)) ?? [];
export const fetchDocVersion = (feature: string, n: number) =>
  getJson<ManualDoc>(`/api/docs/${encodeURIComponent(feature)}/versions/${n}`);

export interface Proposal {
  id: string;
  /** replace | insert_after */
  action: string;
  block: number;
  before: string;
  bid?: string;
  html: string;
  reason: string;
  facts: string[];
}

export const fetchProposals = async (feature: string) =>
  (await getJson<{ version: number; items: Proposal[] }>(`/api/docs/${encodeURIComponent(feature)}/proposals`)) ?? { version: 0, items: [] };

export async function resolveProposal(feature: string, id: string): Promise<void> {
  await fetch(`/api/docs/${encodeURIComponent(feature)}/proposals/${encodeURIComponent(id)}`, { method: "DELETE" });
}

/** Uploads a picture standing in for one old-manual scene (or removes it with `null`). */
export async function setSceneUpload(feature: string, image: string, file: File | null): Promise<SceneShots> {
  const q = `image=${encodeURIComponent(image)}${file ? `&name=${encodeURIComponent(file.name)}` : ""}`;
  const res = await fetch(`/api/references/shots/${encodeURIComponent(feature)}/upload?${q}`, {
    method: file ? "POST" : "DELETE",
    body: file ?? undefined,
  });
  const data = (await res.json().catch(() => ({}))) as SceneShots & { error?: string };
  if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
  return data;
}

export const setSceneChoice = (feature: string, image: string, choice: string | null) =>
  postJson<SceneShots>(`/api/references/shots/${encodeURIComponent(feature)}/choice`, { image, choice });

/** What an SSH server holds (서버 살펴보기). */
export interface SourceProbe {
  containers: { name: string; image: string; ports: string }[];
  ports: number[];
  path_ok: boolean;
  entries: string[];
  /** Site addresses, those mwa's machine can reach first. */
  urls: { url: string; what: string; reachable: boolean }[];
}

export interface SshPlace {
  ssh: string;
  jump: string;
  docker: string;
  remote_path: string;
}

export const probeSource = (src: SshPlace) => postJson<SourceProbe>("/api/sources/probe", src);
/** Checks the route; passwords typed for hops are tried once and saved when it works. */
export const sshCheck = (src: SshPlace, passwords: { hop: string; password: string }[] = []) =>
  postJson<{ who: string; saved: number }>("/api/sources/probe", { ...src, action: "check", passwords });
export const sshList = (src: SshPlace, path: string) =>
  postJson<{ entries: { name: string; dir: boolean }[] }>("/api/sources/probe", { ...src, action: "ls", path });
export const sshGuess = (src: SshPlace) =>
  postJson<{ found: { path: string; kind: string }[] }>("/api/sources/probe", { ...src, action: "guess" });
/** The folder's working state as it will be copied. */
export interface SshGitState {
  repo: boolean;
  branch: string;
  head: string;
  dirty: number;
  ahead: number | null;
  behind: number | null;
}
export const sshGit = (src: SshPlace) => postJson<SshGitState>("/api/sources/probe", { ...src, action: "git" });
export const sshKey = () => postJson<{ public_key: string }>("/api/sources/probe", { action: "key" });

export const setSceneInstruction = (feature: string, image: string, instruction: string | null) =>
  postJson<SceneShots>(`/api/references/shots/${encodeURIComponent(feature)}/instruction`, { image, instruction });

export const linkReference = (feature: string, id: string) => postJson<{ ok: boolean }>("/api/references/link", { feature, id });

/** Uploads a PDF to the engine's project and links it to the feature. */
export async function uploadReference(feature: string, file: File): Promise<RefDoc> {
  const res = await fetch(`/api/references/upload?feature=${encodeURIComponent(feature)}&name=${encodeURIComponent(file.name)}`, {
    method: "POST",
    body: file,
  });
  const data = (await res.json().catch(() => ({}))) as RefDoc & { error?: string };
  if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`);
  return data;
}

export const restoreDoc = (feature: string, n: number) =>
  postJson<ManualDoc>(`/api/docs/${encodeURIComponent(feature)}/restore/${n}`, {});

/** The editor's right-click helper: ask | polish | simpler | shorter | rewrite. */
export interface AssistEdit {
  /** Consecutive blocks (data-bid) replaced by `html`; empty = insert after `after`. */
  replace: string[];
  after: string;
  html: string;
}

export interface AssistReply {
  answer: string;
  edits: AssistEdit[];
  facts: string[];
}

/** The editor's side-panel helper: sees the whole manual, starts from `focus` (block ids in view). */
export const assist = (feature: string, body: { request: string; html: string; focus: string[]; history: { who: string; text: string }[] }) =>
  postJson<AssistReply>(`/api/docs/${encodeURIComponent(feature)}/assist`, body);

export interface ProvEntry {
  kind: string;
  version: number;
  at: number;
  page?: number;
  facts?: string[];
  request?: string;
  summary: string;
}

export interface Provenance {
  blocks: Record<string, ProvEntry[]>;
  log: { version: number; at: number; items: { bid: string; kind: string; summary: string }[] }[];
}

export const fetchProvenance = async (feature: string) =>
  (await getJson<Provenance>(`/api/docs/${encodeURIComponent(feature)}/provenance`)) ?? { blocks: {}, log: [] };

/** Why a block says what it says (last "근거 확인"). */
export interface Ground {
  /** supported | partial | contradicted | unsupported | no_claim */
  verdict: string;
  facts: string[];
  why: string;
  /** Block text when checked. */
  text: string;
  /** The cross-check model's view (D-051); verdict is `disputed` when they disagree. */
  cross?: { model: string; verdict: string; why: string } | null;
}

export interface Grounding {
  version: number;
  at: number;
  blocks: Record<string, Ground>;
}

export const fetchGrounding = async (feature: string) =>
  (await getJson<Grounding>(`/api/docs/${encodeURIComponent(feature)}/grounding`)) ?? { version: 0, at: 0, blocks: {} };

export interface ConfluenceStatus {
  /** The add-on is switched on in settings. */
  enabled?: boolean;
  configured: boolean;
  base_url?: string | null;
  email?: string | null;
  space?: string | null;
  parent?: string | null;
  has_token?: boolean;
  published?: { page_id: string; url: string; title: string; version: number; doc_version: number; at: number; path?: string[] } | null;
  /** Names of the configured place (from the last check). */
  target?: { space_name: string; parent_title: string; parent_url: string } | null;
}

export const fetchConfluence = async (feature: string) =>
  (await getJson<ConfluenceStatus>(`/api/docs/${encodeURIComponent(feature)}/confluence`)) ?? { configured: false };

export const setupConfluence = (body: { page_url: string; email: string; token: string }) =>
  postJson<{ found: string }>("/api/confluence/setup", body);

export const publishConfluence = (
  feature: string,
  body: { title: string; storage: string; images: { name: string; url: string }[]; adopt: boolean; group: string | null },
) => postJson<NonNullable<ConfluenceStatus["published"]>>(`/api/docs/${encodeURIComponent(feature)}/confluence`, body);

/** Where one feature's manual stands (home screen). */
export interface ManualStatus {
  key: string;
  label: string;
  group: string;
  reference: { title: string; pictures: number; described: number } | null;
  scenes: { total: number; reproduced: number; different: number; failed: number; uploaded: number } | null;
  doc: { version: number; updated_at: number; origin: string } | null;
  grounding: { at: number; version: number; supported: number; partial: number; contradicted: number; unsupported: number; stale: boolean } | null;
  code: { read_at: number; facts: number; changed: boolean | null } | null;
  published: { url: string; version: number; doc_version: number } | null;
}

export const fetchManuals = async () =>
  (await getJson<{ features: ManualStatus[]; publish: boolean }>("/api/manuals")) ?? { features: [], publish: false };

/** 자동 작업: a rule keeping manuals up to date on a schedule (D-050). */
export interface AutoRule {
  id: string;
  name: string;
  enabled: boolean;
  every: "daily" | "weekly" | "hours";
  at: string;
  weekday: number;
  hours: number;
  scope: "changed" | "features";
  features: string[];
  action: "propose" | "rewrite";
  reshoot: boolean;
  until: "review" | "publish";
  last_run?: number | null;
  created_at?: number;
  /** Filled by the engine. */
  next_run?: number | null;
}

export interface AutoRun {
  rule: string;
  name: string;
  started: number;
  finished?: number | null;
  status: "running" | "ok" | "none" | "failed";
  features: string[];
  lines: string[];
}

export interface Automation {
  rules: AutoRule[];
  runs: AutoRun[];
  running: boolean;
}

export const fetchAutomation = async () =>
  (await getJson<Automation>("/api/automation")) ?? { rules: [], runs: [], running: false };

export async function saveAutomation(rules: AutoRule[]): Promise<Automation> {
  const res = await fetch("/api/automation", {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ rules: rules.map(({ next_run: _, ...r }) => r) }),
  });
  if (!res.ok) throw new Error(await res.text());
  return res.json();
}
