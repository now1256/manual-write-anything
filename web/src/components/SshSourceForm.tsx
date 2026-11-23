import { useState } from "react";
import { probeSource, sshCheck, sshGuess, sshKey, sshList, type SourceProbe, type SshPlace } from "../api";

interface Props {
  place: SshPlace;
  onChange: (p: Partial<SshPlace>) => void;
  /** Adds an address as a site to shoot. */
  onAddSite: (url: string) => void;
}

interface Hop {
  user: string;
  host: string;
  port: string;
  /** Typed now; sent with the check and saved only when it works. */
  password: string;
  status: { ok: boolean; text: string } | null;
}

type Busy = null | "check" | "probe" | "ls" | "guess" | "key";

const err = (x: unknown) => String((x as Error).message ?? x);

/**
 * Code on another machine, step by step: where to connect (through jump hosts), whether the
 * code is in a container, which folder. Every step can be checked on the spot; nothing is
 * written on the server.
 */
export default function SshSourceForm({ place, onChange, onAddSite }: Props) {
  // The route as rows: jump hosts first, the server with the code last. Each row is checked
  // through the ones before it.
  const parse = (t: string): Hop => {
    const [user, rest] = t.includes("@") ? t.split("@", 2) : ["", t];
    const m = rest.match(/^(.*?):(\d+)$/);
    return { user, host: m ? m[1] : rest, port: m ? m[2] : "", password: "", status: null };
  };
  const [rows, setRows] = useState<Hop[]>(() => {
    const jumps = place.jump ? place.jump.split(",").map((h) => h.trim()).filter(Boolean) : [];
    return [...jumps, place.ssh].map(parse);
  });
  const fmt = (h: Hop) => `${h.user ? `${h.user}@` : ""}${h.host.trim()}${h.port ? `:${h.port}` : ""}`;
  const save = (next: Hop[]) => {
    setRows(next);
    const last = next[next.length - 1];
    onChange({ ssh: fmt(last), jump: next.slice(0, -1).map(fmt).filter(Boolean).join(", ") });
  };
  const edit = (i: number, patch: Partial<Hop>) =>
    // Changing a hop un-checks it and every hop after it.
    save(rows.map((h, k) => (k === i ? { ...h, ...patch, status: null } : k > i ? { ...h, status: null } : h)));
  const routeOk = rows[rows.length - 1].status?.ok === true;
  const hops = rows.slice(0, -1).map(fmt);
  const [busy, setBusy] = useState<Busy>(null);
  const [conn, setConn] = useState<{ ok: boolean; text: string } | null>(null);
  const checkRow = (i: number) => {
    const upto = rows.slice(0, i + 1);
    const passwords = upto.filter((h) => h.password).map((h) => ({ hop: fmt(h), password: h.password }));
    void run(
      "check",
      () => sshCheck({ ssh: fmt(upto[i]), jump: upto.slice(0, -1).map(fmt).join(", "), docker: "", remote_path: "" }, passwords),
      (v) =>
        setRows((all) =>
          all.map((h, k) =>
            k <= i
              ? { ...h, password: "", status: k === i ? { ok: true, text: `접속됨 — ${v.who}${v.saved ? " · 비밀번호 저장함" : ""}` } : h.status ?? { ok: true, text: "" } }
              : h,
          ),
        ),
      (e) => setRows((all) => all.map((h, k) => (k === i ? { ...h, status: { ok: false, text: e } } : h))),
    );
  };
  const [probe, setProbe] = useState<SourceProbe | null>(null);
  const [inDocker, setInDocker] = useState(!!place.docker);
  const [browse, setBrowse] = useState<{ path: string; entries: { name: string; dir: boolean }[] } | null>(null);
  const [found, setFound] = useState<{ path: string; kind: string }[] | null>(null);
  const [key, setKey] = useState<string | null>(null);
  const [note, setNote] = useState<{ ok: boolean; text: string } | null>(null);

  const run = async <T,>(what: Busy, f: () => Promise<T>, done: (v: T) => void, fail: (e: string) => void) => {
    setBusy(what);
    try {
      done(await f());
    } catch (x) {
      fail(err(x));
    } finally {
      setBusy(null);
    }
  };
  const denied = rows.some((h) => h.status && !h.status.ok && /거절|Permission denied/.test(h.status.text));
  const ready = routeOk;

  return (
    <div className="ssh">
      <div className="ssh-route" title="mwa가 코드에 닿는 길">
        <span className="hop me">이 mwa</span>
        {hops.filter(Boolean).map((h, i) => (
          <span key={i} className="hop">→ {h}</span>
        ))}
        <span className="hop">→ {place.ssh || "서버"}</span>
        {inDocker && <span className="hop box">→ 🐳 {place.docker || "컨테이너"}</span>}
        <span className="hop">→ 📁 {place.remote_path || "폴더"}</span>
      </div>

      <section className="ssh-step">
        <h4><span className="no">1</span> 어디로 접속하나요?</h4>
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
          <p className="small">
            <b>비밀번호 대신 접속 키.</b> 아래 키를 각 서버의 접속 계정에 한 번 등록하면 비밀번호 없이 붙습니다.
          </p>
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
              <p className="muted small">대상 서버에 그 계정으로 로그인해 "등록 명령"을 붙여 넣으면 됩니다. 이 키는 mwa 데이터 폴더에만 있고, 사용자의 ~/.ssh는 건드리지 않습니다.</p>
            </>
          ) : (
            <button className="btn small" disabled={busy !== null} onClick={() => run("key", sshKey, (v) => setKey(v.public_key), (e) => setNote({ ok: false, text: e }))}>
              {busy === "key" ? "만드는 중…" : "mwa 접속 키 보기"}
            </button>
          )}
          {note && <p className={`small ${note.ok ? "ok-text" : "err-text"}`}>{note.text}</p>}
        </details>
      </section>

      {!routeOk && <p className="ssh-locked muted small">1단계에서 마지막 서버까지 연결이 확인되면 아래 단계가 열립니다.</p>}
      <fieldset className="ssh-rest" disabled={!routeOk}>
        {conn && !conn.ok && <p className="small err-text">✗ {conn.text}</p>}
      <section className="ssh-step">
        <h4><span className="no">2</span> 코드가 어디에 있나요?</h4>
        <div className="seg">
          <button className={!inDocker ? "on" : ""} onClick={() => { setInDocker(false); onChange({ docker: "" }); }}>서버에 바로</button>
          <button className={inDocker ? "on" : ""} onClick={() => setInDocker(true)}>도커 컨테이너 안</button>
        </div>
        {inDocker && (
          <>
            <div className="ssh-actions">
              <input className="grow" placeholder="컨테이너 이름" value={place.docker} onChange={(e) => onChange({ docker: e.target.value })} />
              <button
                className="btn small"
                disabled={!ready || busy !== null}
                onClick={() => run("probe", () => probeSource(place), setProbe, (e) => setConn({ ok: false, text: e }))}
              >
                {busy === "probe" ? "읽는 중…" : "컨테이너 목록 보기"}
              </button>
            </div>
            {probe && probe.containers.length > 0 && (
              <ul className="ssh-list">
                {probe.containers.map((c) => (
                  <li key={c.name} className={c.name === place.docker ? "on" : ""}>
                    <button className="link" onClick={() => onChange({ docker: c.name })}>
                      <b>{c.name}</b>
                    </button>{" "}
                    <span className="muted small">{c.image}{c.ports ? ` · ${c.ports}` : ""}</span>
                  </li>
                ))}
              </ul>
            )}
            {probe && probe.containers.length === 0 && <p className="muted small">실행 중인 컨테이너를 찾지 못했습니다 (이 계정의 docker 권한 확인).</p>}
          </>
        )}
      </section>

      <section className="ssh-step">
        <h4><span className="no">3</span> 소스 폴더</h4>
        <div className="ssh-actions">
          <input className="grow" placeholder="/app" value={place.remote_path} onChange={(e) => onChange({ remote_path: e.target.value })} />
          <button
            className="btn small"
            disabled={!ready || busy !== null}
            onClick={() => run("guess", () => sshGuess(place), (v) => setFound(v.found), (e) => setConn({ ok: false, text: e }))}
          >
            {busy === "guess" ? "찾는 중…" : "자동으로 찾기"}
          </button>
          <button
            className="btn small"
            disabled={!ready || busy !== null}
            onClick={() => {
              const at = place.remote_path || "/";
              void run("ls", () => sshList(place, at), (v) => setBrowse({ path: at, entries: v.entries }), (e) => setConn({ ok: false, text: e }));
            }}
          >
            찾아보기
          </button>
        </div>
        {found && (
          found.length ? (
            <ul className="ssh-list">
              {found.map((f) => (
                <li key={f.path} className={f.path === place.remote_path ? "on" : ""}>
                  <button className="link" onClick={() => onChange({ remote_path: f.path })}>{f.path}</button> <span className="muted small">{f.kind} 있음</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted small">흔한 위치(/app, /usr/src/app, /code …)에서 코드를 찾지 못했습니다. "찾아보기"로 직접 고르세요.</p>
          )
        )}
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
                  <button className="link" onClick={() => {
                    const up = browse.path.replace(/\/[^/]+\/?$/, "") || "/";
                    void run("ls", () => sshList(place, up), (v) => setBrowse({ path: up, entries: v.entries }), (e) => setConn({ ok: false, text: e }));
                  }}>⬆ 위로</button>
                </li>
              )}
              {browse.entries.map((e) => (
                <li key={e.name}>
                  {e.dir ? (
                    <button className="link" onClick={() => {
                      const next = `${browse.path.replace(/\/$/, "")}/${e.name}`;
                      void run("ls", () => sshList(place, next), (v) => setBrowse({ path: next, entries: v.entries }), (x) => setConn({ ok: false, text: x }));
                    }}>📁 {e.name}</button>
                  ) : (
                    <span className={/^(package\.json|pyproject\.toml|requirements\.txt)$/.test(e.name) ? "ok-text" : "muted"}>📄 {e.name}</span>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}
      </section>

      <section className="ssh-step">
        <h4><span className="no">4</span> 제품이 떠 있는 주소 (선택)</h4>
        <div className="ssh-actions">
          <button
            className="btn small"
            disabled={!ready || busy !== null}
            onClick={() => run("probe", () => probeSource(place), setProbe, (e) => setConn({ ok: false, text: e }))}
          >
            {busy === "probe" ? "읽는 중…" : "열린 포트에서 찾기"}
          </button>
          <span className="muted small">촬영할 사이트 주소 후보입니다. 누르면 "사이트와 계정"에 추가됩니다.</span>
        </div>
        {probe && (
          probe.urls.length ? (
            <ul className="ssh-list">
              {probe.urls.map(([u, what]) => (
                <li key={u}>
                  <button className="link" onClick={() => onAddSite(u)}>{u}</button> <span className="muted small">{what}</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted small">열린 포트를 찾지 못했습니다.</p>
          )
        )}
      </section>

      </fieldset>
      <p className="muted small">서버에서는 읽기만 합니다(목록 보기, 폴더를 묶어 복사). 저장한 뒤 "소스 받기"를 누르면 코드를 가져옵니다.</p>
    </div>
  );
}
