import { useEffect, useState } from "react";
import { fetchSettings, saveSettings, setPassword, type Settings } from "../api";

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

  useEffect(() => {
    fetchSettings()
      .then(setS)
      .catch((e) => setError(String(e.message ?? e)));
  }, []);

  if (error && !s) return <div className="empty error">설정을 불러오지 못했습니다: {error}</div>;
  if (!s) return <div className="empty">불러오는 중…</div>;

  const update = (f: (d: Settings) => void) => {
    const d = structuredClone(s);
    f(d);
    setS(d);
    setNote(null);
  };

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const r = await saveSettings(s);
      setNote(r.note);
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

  return (
    <div className="settings">
      <div className="settings-head">
        <h2>프로젝트 설정</h2>
        <span className="spacer" />
        <button onClick={onClose}>닫기</button>
        <button className="primary" disabled={saving} onClick={save}>
          {saving ? "저장 중…" : "저장"}
        </button>
      </div>
      {error && <p className="settings-error">{error}</p>}
      {note && <p className="settings-note">{note}</p>}

      <section>
        <h3>코드 소스</h3>
        <p className="muted small">로컬 폴더 또는 git 저장소. git만 적으면 프로젝트 안에 클론하고 /sync로 최신으로 맞춥니다. 코드는 읽기만 합니다.</p>
        {s.sources.map((src, i) => (
          <div key={i} className="row4">
            <input placeholder="이름 (예: frontend)" value={src.name} onChange={(e) => update((d) => (d.sources[i].name = e.target.value))} />
            <input placeholder="로컬 폴더 (git이면 비움)" value={src.path} onChange={(e) => update((d) => (d.sources[i].path = e.target.value))} />
            <input placeholder="git 주소" value={src.git} onChange={(e) => update((d) => (d.sources[i].git = e.target.value))} />
            <input placeholder="브랜치" value={src.branch} onChange={(e) => update((d) => (d.sources[i].branch = e.target.value))} />
            <button title="삭제" onClick={() => update((d) => d.sources.splice(i, 1))}>✕</button>
          </div>
        ))}
        <div className="settings-actions">
          <button onClick={() => update((d) => d.sources.push({ name: "", path: "", git: "", branch: "" }))}>소스 추가</button>
          <button onClick={() => onCommand("sync")}>git 소스 동기화 (/sync)</button>
        </div>
      </section>

      <section>
        <h3>제품</h3>
        {s.product ? (
          <div className="grid2">
            <label>제품 이름<input value={s.product.name} onChange={(e) => update((d) => (d.product!.name = e.target.value))} /></label>
            <label>어댑터 파일<input value={s.product.adapter} onChange={(e) => update((d) => (d.product!.adapter = e.target.value))} placeholder="adapter.toml" /></label>
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
            <label className="wide">OpenAPI 주소 또는 파일 (선택)<input value={s.product.openapi} onChange={(e) => update((d) => (d.product!.openapi = e.target.value))} /></label>
          </div>
        ) : (
          <button onClick={() => update((d) => (d.product = { name: "", adapter: "adapter.toml", frontend: "", backend: "", openapi: "" }))}>제품 설정 추가</button>
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
          <button onClick={() => update((d) => (d.capture = { profile: d.profiles[0]?.name ?? "", domain: "", viewport: [1440, 900], scale: 2 }))}>촬영 설정 추가</button>
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
    </div>
  );
}
