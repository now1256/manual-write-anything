import { useEffect, useState } from "react";
import { probeSource, sshCheck, sshGit, sshGuess, sshKey, sshList, type SourceProbe, type SshGitState, type SshPlace } from "../api";

/** One code folder on the server: a source of its own. */
export interface ServerFolder {
  /** Index of the source in the settings. */
  index: number;
  name: string;
  docker: string;
  remote_path: string;
}

interface Props {
  /** `user@host[:port]` of the server with the code, and the hops on the way. */
  ssh: string;
  jump: string;
  folders: ServerFolder[];
  /** Changes the route for every folder on this server. */
  onRoute: (r: { ssh: string; jump: string }) => void;
  onFolder: (index: number, patch: Partial<Pick<ServerFolder, "name" | "docker" | "remote_path">>) => void;
  onAddFolder: () => void;
  onRemoveFolder: (index: number) => void;
  onRemoveServer: () => void;
  /** Adds an address as a site to shoot; returns the account it went into. */
  onAddSite: (url: string) => string;
  /** Addresses already set up as sites to shoot. */
  sites: string[];
}

interface Hop {
  user: string;
  host: string;
  port: string;
  /** Typed now; sent with the check and saved only when it works. */
  password: string;
  status: { ok: boolean; text: string } | null;
}

/** Routes checked in this page, so a server card opened again starts as checked. */
const checked = new Set<string>();
const err = (x: unknown) => String((x as Error).message ?? x);
const parse = (t: string): Hop => {
  const [user, rest] = t.includes("@") ? t.split("@", 2) : ["", t];
  const m = rest.match(/^(.*?):(\d+)$/);
  return { user, host: m ? m[1] : rest, port: m ? m[2] : "", password: "", status: null };
};
const fmt = (h: Hop) => `${h.user ? `${h.user}@` : ""}${h.host.trim()}${h.port ? `:${h.port}` : ""}`;

/**
 * A server holding code: how to reach it (checked hop by hop, through jump hosts), the code
 * folders on it (each in the server itself or in a container), and where the product is
 * served. Nothing on the server is written.
 */
export default function SshServer({ ssh, jump, folders, onRoute, onFolder, onAddFolder, onRemoveFolder, onRemoveServer, onAddSite, sites }: Props) {
  const [rows, setRows] = useState<Hop[]>(() => {
    const jumps = jump ? jump.split(",").map((h) => h.trim()).filter(Boolean) : [];
    const list = [...jumps, ssh].map(parse);
    const key = `${jump}|${ssh}`;
    if (ssh && checked.has(key)) return list.map((h) => ({ ...h, status: { ok: true, text: "이 화면에서 확인한 경로" } }));
    // A saved server opens ready; a person re-checks only after changing the route.
    if (ssh && folders.some((f) => f.remote_path)) return list.map((h) => ({ ...h, status: { ok: true, text: "저장된 경로 (바꿨다면 연결 확인을 다시 누르세요)" } }));
    return list;
  });
  const save = (next: Hop[]) => {
    setRows(next);
    onRoute({ ssh: fmt(next[next.length - 1]), jump: next.slice(0, -1).map(fmt).filter(Boolean).join(", ") });
  };
  const edit = (i: number, patch: Partial<Hop>) =>
    // Changing a hop un-checks it and every hop after it.
    save(rows.map((h, k) => (k === i ? { ...h, ...patch, status: null } : k > i ? { ...h, status: null } : h)));
  const routeOk = rows[rows.length - 1].status?.ok === true;
  // Folded only when the card opens on a route already checked; while editing it stays open.
  const [routeOpen, setRouteOpen] = useState(() => !routeOk);
  const [busy, setBusy] = useState<string | null>(null);
  const run = async <T,>(what: string, f: () => Promise<T>, done: (v: T) => void, fail: (e: string) => void) => {
    setBusy(what);
    try {
      done(await f());
    } catch (x) {
      fail(err(x));
    } finally {
      setBusy(null);
    }
  };
  const checkRow = (i: number) => {
    const upto = rows.slice(0, i + 1);
    const passwords = upto.filter((h) => h.password).map((h) => ({ hop: fmt(h), password: h.password }));
    void run(
      "check",
      () => sshCheck({ ssh: fmt(upto[i]), jump: upto.slice(0, -1).map(fmt).join(", "), docker: "", remote_path: "" }, passwords),
      (v) => {
        if (i === rows.length - 1) checked.add(`${upto.slice(0, -1).map(fmt).join(", ")}|${fmt(upto[i])}`);
        setRows((all) =>
          all.map((h, k) =>
            k <= i
              ? { ...h, password: "", status: k === i ? { ok: true, text: `접속됨 — ${v.who}${v.saved ? " · 비밀번호 저장함" : ""}` } : h.status ?? { ok: true, text: "" } }
              : h,
          ),
        );
      },
      (e) => setRows((all) => all.map((h, k) => (k === i ? { ...h, status: { ok: false, text: e } } : h))),
    );
  };

  // What the server holds (containers, addresses), read once the route works.
  const [probe, setProbe] = useState<SourceProbe | null>(null);
  const [probeErr, setProbeErr] = useState<string | null>(null);
  const route: SshPlace = { ssh, jump, docker: "", remote_path: "/" };
  useEffect(() => {
    if (!routeOk || !ssh) return;
    let live = true;
    probeSource(route)
      .then((p) => live && setProbe(p))
      .catch((e) => live && setProbeErr(err(e)));
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [routeOk, ssh, jump]);

  const [key, setKey] = useState<string | null>(null);
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null);
  const [siteNote, setSiteNote] = useState<string | null>(null);
  const denied = rows.some((h) => h.status && !h.status.ok && /거절|Permission denied/.test(h.status.text));

  return (
    <div className="server-card">
      <div className="server-head">
        <span className="server-icon">🖥</span>
        <b>{ssh || "새 서버"}</b>
        {rows.length > 1 && <span className="muted small">경유 {rows.slice(0, -1).map(fmt).join(" → ")}</span>}
        <span className={`small ${routeOk ? "ok-text" : "muted"}`}>{routeOk ? "✓ 접속됨" : "접속 확인 전"}</span>
        <span className="spacer" />
        <button className="link small" onClick={onRemoveServer} title="이 서버와 그 폴더들을 뺍니다">
          서버 빼기
        </button>
      </div>

      <details className="server-route" open={routeOpen} onToggle={(e) => setRouteOpen((e.target as HTMLDetailsElement).open)}>
        <summary>접속 경로 {routeOk ? "(확인됨 — 펼쳐서 바꾸기)" : ""}</summary>
        <p className="muted small">
          처음 접속할 서버부터 하나씩 연결을 확인합니다. 그 서버를 거쳐 다른 서버(또는 그 안의 컨테이너 SSH)로 들어가야 하면, 확인된 뒤 "한 단계 더 들어가기"를 누릅니다.
          이 서버의 <code>~/.ssh/config</code>에 별칭(Host)이 있으면 주소에 별칭만 적어도 됩니다.
        </p>
        {rows.map((h, i) => (
          <div key={i} className={`ssh-hoprow ${h.status?.ok ? "ok" : h.status ? "bad" : ""}`}>
            <div className="ssh-hoplabel">
              {i === 0 ? "처음 접속할 서버" : `${i + 1}단계 — 위 서버를 거쳐서`}
              {i === rows.length - 1 && rows.length > 1 && <span className="muted"> (코드가 있는 서버)</span>}
            </div>
            <div className="ssh-target">
              <label>
                사용자
                <input placeholder="root" value={h.user} onChange={(e) => edit(i, { user: e.target.value.trim() })} />
              </label>
              <span className="at">@</span>
              <label className="grow">
                주소 {i > 0 && <span className="muted">(위 서버 기준, 예: localhost)</span>}
                <input placeholder={i === 0 ? "10.0.0.5 또는 별칭" : "localhost"} value={h.host} onChange={(e) => edit(i, { host: e.target.value })} />
              </label>
              <label className="port">
                포트
                <input placeholder="22" inputMode="numeric" value={h.port} onChange={(e) => edit(i, { port: e.target.value.replace(/\D/g, "") })} />
              </label>
              <label className="pw">
                비밀번호 <span className="muted">(키가 없으면)</span>
                <input type="password" autoComplete="new-password" value={h.password} onChange={(e) => setRows((all) => all.map((x, k) => (k === i ? { ...x, password: e.target.value } : x)))} />
              </label>
              <button className="btn primary" disabled={!h.host.trim() || busy !== null || (i > 0 && !rows[i - 1].status?.ok)} onClick={() => checkRow(i)}>
                {busy === "check" ? "확인 중…" : "연결 확인"}
              </button>
              {i > 0 && (
                <button className="link" onClick={() => save(rows.filter((_, k) => k !== i))}>
                  빼기
                </button>
              )}
            </div>
            {h.status && <div className={`small ${h.status.ok ? "ok-text" : "err-text"}`}>{h.status.ok ? "✓ " : "✗ "}{h.status.text || "접속됨"}</div>}
          </div>
        ))}
        <div className="ssh-actions">
          <button
            className="btn"
            disabled={!routeOk}
            title={routeOk ? "" : "마지막 서버의 연결을 먼저 확인하세요"}
            onClick={() => setRows((all) => [...all, { user: all[all.length - 1].user, host: "localhost", port: "", password: "", status: null }])}
          >
            + 이 서버를 거쳐 한 단계 더 들어가기
          </button>
          <span className="muted small">비밀번호는 연결이 성공했을 때만 사용자 자격 파일에 저장됩니다. 서버마다 1번만 시도해 계정이 잠기지 않습니다.</span>
        </div>
        <details className="ssh-help" open={denied}>
          <summary>비밀번호 대신 접속 키를 쓰려면</summary>
          <p className="small">아래 키를 각 서버의 접속 계정에 한 번 등록하면 비밀번호 없이 붙습니다. 이 키는 mwa 데이터 폴더에만 있습니다.</p>
          {key ? (
            <>
              <pre className="ssh-key">{key}</pre>
              <div className="ssh-actions">
                <button className="btn small" onClick={() => void navigator.clipboard.writeText(key).then(() => setNote({ ok: true, text: "키를 복사했습니다" }))}>
                  키 복사
                </button>
                <button
                  className="btn small"
                  onClick={() =>
                    void navigator.clipboard
                      .writeText(`mkdir -p ~/.ssh && echo '${key}' >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys`)
                      .then(() => setNote({ ok: true, text: "등록 명령을 복사했습니다. 대상 서버에서 그 계정으로 붙여 넣어 실행하세요" }))
                  }
                >
                  등록 명령 복사
                </button>
              </div>
            </>
          ) : (
            <button className="btn small" disabled={busy !== null} onClick={() => run("key", sshKey, (v) => setKey(v.public_key), (e) => setNote({ ok: false, text: e }))}>
              {busy === "key" ? "만드는 중…" : "mwa 접속 키 보기"}
            </button>
          )}
          {note && <p className={`small ${note.ok ? "ok-text" : "err-text"}`}>{note.text}</p>}
        </details>
      </details>

      <fieldset className="ssh-rest" disabled={!routeOk}>
        {!routeOk && <p className="muted small">접속 경로가 확인되면 이 서버의 코드 폴더를 고를 수 있습니다.</p>}
        <div className="server-folders">
          <div className="server-folders-head">
            <b>코드 폴더</b>
            <span className="muted small">이 서버의 폴더를 여러 개 넣을 수 있습니다 (예: 프론트, 백엔드).</span>
          </div>
          {folders.map((f) => (
            <FolderRow
              key={f.index}
              place={{ ssh, jump, docker: f.docker, remote_path: f.remote_path }}
              name={f.name}
              ready={routeOk}
              containers={probe?.containers ?? []}
              onChange={(patch) => onFolder(f.index, patch)}
              onRemove={() => onRemoveFolder(f.index)}
            />
          ))}
          <button className="btn" onClick={onAddFolder}>
            + 이 서버에 코드 폴더 추가
          </button>
        </div>

        <div className="server-sites">
          <b>제품이 떠 있는 주소</b> <span className="muted small">mwa가 화면을 찍을 사이트 후보입니다.</span>
          {probeErr && <p className="small err-text">✗ {probeErr}</p>}
          {!probe && routeOk && !probeErr && <p className="muted small">서버를 살펴보는 중…</p>}
          {siteNote && <p className="small ok-text">{siteNote}</p>}
          {probe &&
            (probe.urls.length ? (
              <ul className="ssh-list">
                {probe.urls.map(({ url: u, what, reachable }) => {
                  const added = sites.includes(u);
                  return (
                    <li key={u} className={added ? "on" : ""}>
                      <span className="mono">{u}</span> <span className="muted small">{what}</span>{" "}
                      <span className={`small ${reachable ? "ok-text" : "err-text"}`}>{reachable ? "✓ 이 mwa에서 열림" : "✗ 이 mwa에서 닿지 않음"}</span>{" "}
                      <button
                        className="btn small"
                        disabled={added}
                        onClick={() => {
                          const name = onAddSite(u);
                          setSiteNote(`✓ ${u}을(를) 촬영 사이트로 추가했습니다 (${name}). 저장한 뒤 "사이트와 계정"에서 아이디와 비밀번호를 넣으세요.`);
                        }}
                      >
                        {added ? "✓ 추가됨" : "촬영 사이트로 추가"}
                      </button>{" "}
                      <a className="small" href={u} target="_blank" rel="noreferrer">
                        열어 보기 ↗
                      </a>
                    </li>
                  );
                })}
              </ul>
            ) : (
              <p className="muted small">열린 포트를 찾지 못했습니다.</p>
            ))}
        </div>
      </fieldset>
    </div>
  );
}

/** One code folder on the server: its name, container (if any), path and working state. */
function FolderRow({
  place,
  name,
  ready,
  containers,
  onChange,
  onRemove,
}: {
  place: SshPlace;
  name: string;
  ready: boolean;
  containers: { name: string; image: string; ports: string }[];
  onChange: (patch: Partial<Pick<ServerFolder, "name" | "docker" | "remote_path">>) => void;
  onRemove: () => void;
}) {
  const [inDocker, setInDocker] = useState(!!place.docker);
  const [busy, setBusy] = useState<string | null>(null);
  const [found, setFound] = useState<{ path: string; kind: string }[] | null>(null);
  const [browse, setBrowse] = useState<{ path: string; entries: { name: string; dir: boolean }[] } | null>(null);
  const [git, setGit] = useState<SshGitState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const run = async <T,>(what: string, f: () => Promise<T>, done: (v: T) => void) => {
    setBusy(what);
    setError(null);
    try {
      done(await f());
    } catch (x) {
      setError(err(x));
    } finally {
      setBusy(null);
    }
  };
  const ls = (path: string) => run("ls", () => sshList(place, path), (v) => setBrowse({ path, entries: v.entries }));
  // The folder's working state, read when it settles.
  useEffect(() => {
    setGit(null);
    if (!ready || !place.remote_path.trim()) return;
    let live = true;
    const t = setTimeout(() => {
      sshGit(place)
        .then((g) => live && setGit(g))
        .catch(() => {});
    }, 600);
    return () => {
      live = false;
      clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, place.ssh, place.jump, place.docker, place.remote_path]);

  return (
    <div className="folder-row">
      <div className="ssh-actions">
        <span>📁</span>
        <input className="name" placeholder="이름 (예: frontend)" value={name} onChange={(e) => onChange({ name: e.target.value })} />
        <div className="seg">
          <button className={!inDocker ? "on" : ""} onClick={() => { setInDocker(false); onChange({ docker: "" }); }}>서버에 바로</button>
          <button className={inDocker ? "on" : ""} onClick={() => setInDocker(true)}>도커 컨테이너 안</button>
        </div>
        {inDocker &&
          (containers.length ? (
            <select value={place.docker} onChange={(e) => onChange({ docker: e.target.value })}>
              <option value="">컨테이너 고르기</option>
              {containers.map((c) => (
                <option key={c.name} value={c.name}>
                  {c.name} ({c.image})
                </option>
              ))}
            </select>
          ) : (
            <input placeholder="컨테이너 이름" value={place.docker} onChange={(e) => onChange({ docker: e.target.value })} />
          ))}
        <span className="spacer" />
        <button className="link small" onClick={onRemove}>폴더 빼기</button>
      </div>
      <div className="ssh-actions">
        <input className="grow" placeholder="/app" value={place.remote_path} onChange={(e) => onChange({ remote_path: e.target.value })} />
        <button className="btn small" disabled={!ready || busy !== null} onClick={() => run("guess", () => sshGuess(place), (v) => setFound(v.found))}>
          {busy === "guess" ? "찾는 중…" : "자동으로 찾기"}
        </button>
        <button className="btn small" disabled={!ready || busy !== null} onClick={() => void ls(place.remote_path || "/")}>
          찾아보기
        </button>
      </div>
      {error && <p className="small err-text">✗ {error}</p>}
      {found &&
        (found.length ? (
          <ul className="ssh-list">
            {found.map((f) => (
              <li key={f.path} className={f.path === place.remote_path ? "on" : ""}>
                <button className="link" onClick={() => { onChange({ remote_path: f.path, ...(name ? {} : { name: f.path.split("/").pop() ?? "" }) }); setFound(null); }}>
                  {f.path}
                </button>{" "}
                <span className="muted small">{f.kind}</span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted small">흔한 위치(/root/*, /home/*, /app …)에서 코드를 찾지 못했습니다. "찾아보기"로 직접 고르세요.</p>
        ))}
      {browse && (
        <div className="ssh-browse">
          <div className="ssh-browse-head">
            <b className="mono">{browse.path}</b>
            <span className="spacer" />
            <button className="btn small primary" onClick={() => { onChange({ remote_path: browse.path }); setBrowse(null); }}>이 폴더로</button>
            <button className="link" onClick={() => setBrowse(null)}>닫기</button>
          </div>
          <ul>
            {browse.path !== "/" && (
              <li>
                <button className="link" onClick={() => void ls(browse.path.replace(/\/[^/]+\/?$/, "") || "/")}>⬆ 위로</button>
              </li>
            )}
            {browse.entries.map((e) => (
              <li key={e.name}>
                {e.dir ? (
                  <button className="link" onClick={() => void ls(`${browse.path.replace(/\/$/, "")}/${e.name}`)}>📁 {e.name}</button>
                ) : (
                  <span className={/^(package\.json|pyproject\.toml|requirements\.txt)$/.test(e.name) ? "ok-text" : "muted"}>📄 {e.name}</span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
      {git && (
        <p className="small ssh-git">
          {git.repo ? (
            <>
              <b>가져올 상태</b> 브랜치 <code>{git.branch}</code> · 커밋 <code>{git.head}</code>
              {git.dirty > 0 && <> · <b>커밋 안 한 변경 {git.dirty}개</b></>}
              {git.ahead != null && git.ahead > 0 && <> · 원격보다 <b>{git.ahead}커밋 앞섬</b></>}
              {git.behind != null && git.behind > 0 && <> · 원격보다 {git.behind}커밋 뒤처짐</>}
              {" "}→ 지금 이 폴더 상태 그대로 가져옵니다.
            </>
          ) : (
            <>Git 저장소가 아닌 폴더입니다. 들어 있는 파일 그대로 가져옵니다.</>
          )}
        </p>
      )}
    </div>
  );
}
