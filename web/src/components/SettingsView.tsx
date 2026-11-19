import { useEffect, useState } from "react";
import { checkField, fetchSettings, saveSettings, setPassword, type Settings } from "../api";
import Checked from "./Checked";
import PathPicker from "./PathPicker";
import ReferenceSettings from "./ReferenceSettings";

type Picker = { mode: "dir" | "file"; ext?: string; start?: string; onPick: (p: string) => void };

interface Props {
  onClose: () => void;
  onCommand: (name: string) => void;
}

const MODELS = ["haiku", "sonnet", "opus", "fable"];

/** Project settings (mwa.toml): sources, product, login/capture target, models, budget. */
export default function SettingsView({ onClose, onCommand }: Props) {
  const [s, setS] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [pw, setPw] = useState<Record<string, string>>({});
  const [picker, setPicker] = useState<Picker | null>(null);
  // Per source row: connect by local folder or by git (clone).
  const [gitMode, setGitMode] = useState<Record<number, boolean>>({});
  const [dirty, setDirty] = useState(false);
  const [closing, setClosing] = useState(false);

  useEffect(() => {
    fetchSettings()
      .then((v) => {
        setS(v);
        setGitMode(Object.fromEntries(v.sources.map((x, i) => [i, !!x.git && !x.path])));
      })
      .catch((e) => setError(String(e.message ?? e)));
  }, []);

  if (error && !s) return <div className="empty error">설정을 불러오지 못했습니다: {error}</div>;
  if (!s) return <div className="empty">불러오는 중…</div>;

  const update = (f: (d: Settings) => void) => {
    const d = structuredClone(s);
    f(d);
    setS(d);
    setNote(null);
    setDirty(true);
  };

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const r = await saveSettings(s);
      setNote(r.note);
      setDirty(false);
      setTimeout(() => window.location.reload(), 900);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const savePassword = async (profile: string) => {
    setError(null);
    try {
      const r = await setPassword(profile, pw[profile] ?? "");
      setPw((p) => ({ ...p, [profile]: "" }));
      setNote(`${profile}: ${r.note}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const sourceNames = s.sources.map((x) => x.name).filter(Boolean);

  // Guess front/back from what each saved source holds (Next.js / FastAPI).
  const addProduct = async () => {
    const kinds = await Promise.all(
      sourceNames.map((n) => checkField("source", n).then((c) => [n, c.ok ? c.detail : ""] as const).catch(() => [n, ""] as const)),
    );
    const pick = (re: RegExp) => kinds.find(([, k]) => re.test(k))?.[0] ?? "";
    const only = s.known_adapters.length === 1 ? s.known_adapters[0][1] : "";
    update((d) => {
      d.product = {
        name: "",
        adapter: only,
        frontend: pick(/프론트|JavaScript/),
        backend: pick(/백엔드|Python/),
        openapi: "",
      };
    });
  };

  return (
    <div className="settings">
      <div className="settings-head">
        <h2>프로젝트 설정</h2>
        <span className="spacer" />
        {dirty && <span className="badge warn">저장하지 않은 변경</span>}
        <button
          onClick={() => {
            if (dirty && !closing) {
              setClosing(true);
              setError("저장하지 않은 변경이 있습니다. 저장하려면 \"저장\", 버리려면 \"닫기\"를 한 번 더 누르세요.");
              return;
            }
            onClose();
          }}
        >
          닫기
        </button>
        <button className="primary" disabled={saving} onClick={save}>
          {saving ? "저장 중…" : "저장"}
        </button>
        {(error || note) && (
          <div className={`settings-flash ${error ? "bad" : "ok"}`}>{error ?? note}</div>
        )}
      </div>

      <section>
        <h3>코드 소스</h3>
        <p className="muted small">프론트·백엔드 코드를 연결합니다. 이미 있는 폴더(git 저장소)를 고르거나, Git 주소를 적으면 프로젝트 안에 받아 옵니다. 코드는 읽기만 합니다.</p>
        {s.sources.map((src, i) => (
          <div key={i} className="source-card">
            <div className="source-row">
              <input className="name" placeholder="이름 (예: frontend)" value={src.name} onChange={(e) => update((d) => (d.sources[i].name = e.target.value))} />
              <div className="seg">
                <button className={!gitMode[i] ? "on" : ""} onClick={() => setGitMode((m) => ({ ...m, [i]: false }))}>폴더</button>
                <button className={gitMode[i] ? "on" : ""} onClick={() => setGitMode((m) => ({ ...m, [i]: true }))}>Git</button>
              </div>
              <button title="삭제" onClick={() => update((d) => d.sources.splice(i, 1))}>✕</button>
            </div>
            {!gitMode[i] ? (
              <>
                <div className="source-row">
                  <input className="grow" placeholder="/코드/폴더/경로" value={src.path} onChange={(e) => update((d) => ((d.sources[i].path = e.target.value), (d.sources[i].git = "")))} />
                  <button onClick={() => setPicker({ mode: "dir", start: src.path, onPick: (p) => update((d) => { d.sources[i].path = p; d.sources[i].git = ""; if (!d.sources[i].name) d.sources[i].name = p.split("/").pop() ?? ""; }) })}>찾아보기</button>
                </div>
                <Checked kind="folder" value={src.path} />
              </>
            ) : (
              <>
                <div className="source-row">
                  <input className="grow" placeholder="git 주소 (git@github.com:org/repo.git 또는 https://…)" value={src.git} onChange={(e) => update((d) => ((d.sources[i].git = e.target.value), (d.sources[i].path = "")))} />
                  <input className="branch" placeholder="브랜치 (예: main)" value={src.branch} onChange={(e) => update((d) => (d.sources[i].branch = e.target.value))} />
                </div>
                <Checked kind="git" value={src.git} extra={src.branch} />
              </>
            )}
          </div>
        ))}
        <div className="settings-actions">
          <button onClick={() => { update((d) => d.sources.push({ name: "", path: "", git: "", branch: "" })); }}>소스 추가</button>
          {s.sources.some((x) => x.git) && <button onClick={() => onCommand("sync")}>Git 소스 받기 (/sync, 저장 후)</button>}
        </div>
      </section>

      <section>
        <h3>제품</h3>
        {s.product ? (
          <div className="grid2">
            <label>제품 이름<input value={s.product.name} onChange={(e) => update((d) => (d.product!.name = e.target.value))} /></label>
            <label>어댑터 (제품별 규칙: 메뉴 파일 위치, 로그인 방식 등)
              {s.known_adapters.length > 0 && (
                <select value={s.known_adapters.some(([, p]) => p === s.product!.adapter) ? s.product.adapter : ""} onChange={(e) => e.target.value && update((d) => (d.product!.adapter = e.target.value))}>
                  <option value="">등록된 어댑터에서 고르기</option>
                  {s.known_adapters.map(([n, p]) => (
                    <option key={p} value={p}>{n}</option>
                  ))}
                </select>
              )}
              <span className="inline grow-input">
                <input value={s.product.adapter} onChange={(e) => update((d) => (d.product!.adapter = e.target.value))} placeholder="찾아보기로 어댑터(.toml) 파일을 고르세요" />
                <button onClick={() => setPicker({ mode: "file", ext: "toml", start: s.product!.adapter, onPick: (p) => update((d) => (d.product!.adapter = p)) })}>찾아보기</button>
              </span>
              {s.product.adapter && <Checked kind="adapter" value={s.product.adapter} />}
            </label>
            <label>프론트 소스
              <select value={s.product.frontend} onChange={(e) => update((d) => (d.product!.frontend = e.target.value))}>
                <option value="">선택</option>
                {sourceNames.map((n) => <option key={n}>{n}</option>)}
              </select>
            </label>
            <label>백엔드 소스
              <select value={s.product.backend} onChange={(e) => update((d) => (d.product!.backend = e.target.value))}>
                <option value="">선택</option>
                {sourceNames.map((n) => <option key={n}>{n}</option>)}
              </select>
            </label>
            <label className="wide">OpenAPI 주소 또는 파일 (선택)
              <input value={s.product.openapi} onChange={(e) => update((d) => (d.product!.openapi = e.target.value))} placeholder="http://백엔드/openapi.json" />
              <Checked kind="openapi" value={s.product.openapi} />
            </label>
          </div>
        ) : (
          <button onClick={addProduct}>제품 설정 추가</button>
        )}
      </section>

      <section>
        <h3>로그인 계정과 촬영 대상</h3>
        <p className="muted small">비밀번호는 이 프로젝트가 아니라 사용자 자격 파일(권한 600)에만 저장합니다. 로그인은 1번만 시도하고 실패하면 멈춥니다.</p>
        {s.profiles.map((p, i) => (
          <div key={i} className="row4">
            <input placeholder="프로파일 이름" value={p.name} onChange={(e) => update((d) => (d.profiles[i].name = e.target.value))} />
            <input placeholder="사이트 주소 (http://…)" value={p.base_url} onChange={(e) => update((d) => (d.profiles[i].base_url = e.target.value))} />
            <input placeholder="아이디" value={p.user_id} onChange={(e) => update((d) => (d.profiles[i].user_id = e.target.value))} />
            <select value={p.purpose} onChange={(e) => update((d) => (d.profiles[i].purpose = e.target.value))}>
              <option value="capture">촬영용</option>
              <option value="bootstrap">계정 준비 전용</option>
            </select>
            <button title="삭제" onClick={() => update((d) => d.profiles.splice(i, 1))}>✕</button>
            {p.base_url && <div className="span-all"><Checked kind="site" value={p.base_url} /></div>}
            <div className="pw">
              <input type="password" autoComplete="new-password" placeholder="비밀번호 (저장된 프로파일만)" value={pw[p.name] ?? ""} onChange={(e) => setPw((x) => ({ ...x, [p.name]: e.target.value }))} />
              <button disabled={!pw[p.name]} onClick={() => savePassword(p.name)}>비밀번호 저장</button>
            </div>
          </div>
        ))}
        <div className="settings-actions">
          <button onClick={() => update((d) => d.profiles.push({ name: "", base_url: "", user_id: "", purpose: "capture" }))}>계정 추가</button>
        </div>
        {s.capture ? (
          <div className="grid2">
            <label>촬영 계정
              <select value={s.capture.profile} onChange={(e) => update((d) => (d.capture!.profile = e.target.value))}>
                <option value="">선택</option>
                {s.profiles.map((p) => <option key={p.name}>{p.name}</option>)}
              </select>
            </label>
            <label>도메인·테넌트 (선택)<input value={s.capture.domain} onChange={(e) => update((d) => (d.capture!.domain = e.target.value))} /></label>
            <label>화면 크기
              <span className="inline">
                <input type="number" value={s.capture.viewport[0]} onChange={(e) => update((d) => (d.capture!.viewport[0] = Number(e.target.value)))} />
                ×
                <input type="number" value={s.capture.viewport[1]} onChange={(e) => update((d) => (d.capture!.viewport[1] = Number(e.target.value)))} />
              </span>
            </label>
            <label>배율 (레티나 2)<input type="number" step="0.5" value={s.capture.scale} onChange={(e) => update((d) => (d.capture!.scale = Number(e.target.value)))} /></label>
          </div>
        ) : (
          <button onClick={() => update((d) => (d.capture = { profile: d.profiles[0]?.name ?? "", domain: "", viewport: [1440, 900], scale: 2, node: "" }))}>촬영 설정 추가</button>
        )}
      </section>

      <section>
        <h3>매뉴얼 양식</h3>
        <p className="muted small">처음 생성할 때 이 견본 매뉴얼(PDF 또는 Markdown)에서 절 구성, 표·흐름도·주의 상자 형식, 캡처 배치와 문체를 배웁니다. 한 번 배우면 프로젝트에 저장된 양식을 씁니다.</p>
        <label className="budget wide-input">
          견본 매뉴얼 파일 (서버 경로)
          <span className="inline grow-input">
            <input value={s.template} placeholder="/경로/견본매뉴얼.pdf 또는 .md" onChange={(e) => update((d) => (d.template = e.target.value))} />
            <button onClick={() => setPicker({ mode: "file", ext: "md,pdf", start: s.template, onPick: (p) => update((d) => (d.template = p)) })}>찾아보기</button>
          </span>
          {s.template && <Checked kind="file" value={s.template} />}
        </label>
      </section>

      <ReferenceSettings />

      <section>
        <h3>실행 파일</h3>
        <p className="muted small">비워 두면 PATH와 흔한 설치 위치(~/.local/bin, Homebrew 등)에서 자동으로 찾습니다. 다른 위치에 설치했다면 전체 경로를 적으세요.</p>
        {s.agents.map((a, i) => (
          <label key={a.backend} className="budget wide-input">
            Claude CLI ({a.backend})
            <span className="inline grow-input">
              <input value={a.command} placeholder="자동 (claude)" onChange={(e) => update((d) => (d.agents[i].command = e.target.value))} />
              <button onClick={() => setPicker({ mode: "file", ext: "*", start: a.command, onPick: (p) => update((d) => (d.agents[i].command = p)) })}>찾아보기</button>
            </span>
            <Checked kind="command" value={a.command || "claude"} />
          </label>
        ))}
        {s.capture && (
          <label className="budget wide-input">
            Node.js (촬영기)
            <input value={s.capture.node} placeholder="자동 (node)" onChange={(e) => update((d) => (d.capture!.node = e.target.value))} />
            <Checked kind="command" value={s.capture.node || "node"} />
          </label>
        )}
      </section>

      <section>
        <h3>모델</h3>
        <p className="muted small">역할마다 백엔드와 모델을 고릅니다. 검증은 작성과 다른 계열을 권장합니다.</p>
        <table className="roles">
          <thead><tr><th>역할</th><th>쓰이는 곳</th><th>백엔드</th><th>모델</th></tr></thead>
          <tbody>
            {s.roles.map((r, i) => (
              <tr key={r.role}>
                <td className="mono">{r.role}</td>
                <td className="muted">{r.uses}</td>
                <td>
                  <select value={r.backend} onChange={(e) => update((d) => (d.roles[i].backend = e.target.value))}>
                    {s.backends.map((b) => <option key={b}>{b}</option>)}
                  </select>
                </td>
                <td>
                  <input list="models" value={r.model} onChange={(e) => update((d) => (d.roles[i].model = e.target.value))} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <datalist id="models">{MODELS.map((m) => <option key={m} value={m} />)}</datalist>
        <label className="budget">
          1회 실행 사용량 상한 (정가 환산 USD, 청구 아님)
          <input type="number" step="0.5" value={s.max_usd_per_run} onChange={(e) => update((d) => (d.max_usd_per_run = Number(e.target.value)))} />
        </label>
      </section>
      <section>
        <h3>확장 기능</h3>
        <p className="muted small">기본 작업(매뉴얼 버전 관리, 기존 매뉴얼 갱신, 새 매뉴얼 쓰기) 밖의 연결입니다. 켠 것만 화면에 나타납니다.</p>
        <label className="check">
          <input type="checkbox" checked={s.confluence} onChange={(e) => update((d) => (d.confluence = e.target.checked))} /> 컨플루언스 Cloud에 게시 — ④ 편집·버전에 "컨플루언스에 게시" 버튼 (게시 위치와 API 토큰은 그 창에서 설정)
        </label>
      </section>
      {picker && (
        <PathPicker
          mode={picker.mode}
          ext={picker.ext}
          start={picker.start}
          onClose={() => setPicker(null)}
          onPick={(p) => {
            picker.onPick(p);
            setPicker(null);
          }}
        />
      )}
    </div>
  );
}
