import { useEffect, useState } from "react";
import { fetchConfluence, publishConfluence, setupConfluence, type ConfluenceStatus } from "../api";
import { toStorage } from "../confluenceStorage";

interface Props {
  featureKey: string;
  title: string;
  /** Menu group of the feature ("소스"); new pages can go under a page of that name. */
  group: string | null;
  /** Editor HTML to publish. */
  html: () => string;
  srcOf: (fig: HTMLElement) => string | null;
  dirty: boolean;
  onClose: () => void;
}

/** Publish the manual to Confluence Cloud as a native page (no model calls). */
export default function ConfluencePublish({ featureKey, title, group, html, srcOf, dirty, onClose }: Props) {
  const [byGroup, setByGroup] = useState(!!group);
  const [st, setSt] = useState<ConfluenceStatus | null>(null);
  const [setup, setSetup] = useState(false);
  const [pageUrl, setPageUrl] = useState("");
  const [email, setEmail] = useState("");
  const [token, setToken] = useState("");
  const [name, setName] = useState(title);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string; link?: string } | null>(null);
  const [conflict, setConflict] = useState<string | null>(null);
  const [showXml, setShowXml] = useState(false);

  const load = () =>
    fetchConfluence(featureKey).then((s) => {
      setSt(s);
      setSetup(!s.configured || !s.has_token);
      if (s.email) setEmail(s.email);
      if (s.published?.title) setName(s.published.title);
    });
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [featureKey]);

  const converted = toStorage(html(), srcOf, title);
  const count = (re: RegExp) => (converted.storage.match(re) ?? []).length;

  const saveSetup = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const r = await setupConfluence({ page_url: pageUrl, email, token });
      setMsg({ ok: true, text: r.found });
      setToken("");
      await load();
      setSetup(false);
    } catch (x) {
      setMsg({ ok: false, text: String((x as Error).message ?? x) });
    } finally {
      setBusy(false);
    }
  };

  const publish = async (adopt: boolean) => {
    setBusy(true);
    setMsg(null);
    setConflict(null);
    try {
      const p = await publishConfluence(featureKey, {
        title: name,
        storage: converted.storage,
        images: converted.images,
        adopt,
        group: byGroup ? group : null,
      });
      setMsg({ ok: true, text: `게시했습니다 (컨플루언스 버전 ${p.version})`, link: p.url });
      await load();
    } catch (x) {
      const t = String((x as Error).message ?? x);
      const m = t.match(/SAME_TITLE (\S+)/);
      if (m) setConflict(m[1]);
      else setMsg({ ok: false, text: t });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="picker-backdrop" onMouseDown={onClose}>
      <div className="cf" onMouseDown={(e) => e.stopPropagation()}>
        <div className="cf-head">
          <b>컨플루언스에 게시</b>
          <span className="spacer" />
          <button className="notice-x" onClick={onClose}>×</button>
        </div>
        {!st ? (
          <p className="muted">불러오는 중…</p>
        ) : setup ? (
          <div className="cf-form">
            <p className="small">
              문서가 들어갈 <b>상위 페이지</b>를 컨플루언스에서 열고 주소를 그대로 붙여 넣으세요. 이 아래에 기능별 페이지를 만들고, 다음부터는 같은 페이지를 갱신합니다.
            </p>
            <label>
              상위 페이지 주소
              <input value={pageUrl} onChange={(e) => setPageUrl(e.target.value)} placeholder="https://회사.atlassian.net/wiki/spaces/DOC/pages/123456/사용자-매뉴얼" />
            </label>
            <label>
              로그인 이메일
              <input value={email} onChange={(e) => setEmail(e.target.value)} placeholder="name@company.com" />
            </label>
            <label>
              API 토큰 {st.has_token && <span className="muted small">(비워 두면 저장된 토큰 사용)</span>}
              <input type="password" value={token} onChange={(e) => setToken(e.target.value)} placeholder="id.atlassian.com에서 만든 토큰" />
            </label>
            <p className="muted small">
              토큰 만들기: <span className="mono">https://id.atlassian.com/manage-profile/security/api-tokens</span> → "API 토큰 만들기". 토큰은 이 엔진의 자격 파일(~/.config/mwa/credentials.json, 0600)에만 저장되고 프로젝트·저장소에는 남지 않습니다.
            </p>
            <div className="cf-actions">
              {st.configured && st.has_token && <button className="btn" onClick={() => setSetup(false)}>취소</button>}
              <button className="btn primary" disabled={busy || !pageUrl || !email || (!token && !st.has_token)} onClick={saveSetup}>
                {busy ? "확인하는 중…" : "연결 확인 후 저장"}
              </button>
            </div>
          </div>
        ) : (
          <div className="cf-form">
            <div className="cf-where">
              <div className="cf-where-head">
                <b>올라갈 위치</b>
                <span className="spacer" />
                <button className="link" onClick={() => setSetup(true)}>위치 바꾸기</button>
              </div>
              {st.published ? (
                <>
                  <ol className="crumbs">
                    {(st.published.path?.length ? st.published.path : [st.target?.space_name ?? st.space ?? "", st.target?.parent_title ?? "", st.published.title])
                      .filter(Boolean)
                      .map((p, i, all) => (
                        <li key={i} className={i === all.length - 1 ? "here" : ""}>{p}</li>
                      ))}
                  </ol>
                  <p className="small">
                    <b>이미 있는 페이지를 갱신합니다</b> (지금 컨플루언스 버전 {st.published.version} → {st.published.version + 1}). 위치는 바뀌지 않습니다.{" "}
                    <a href={st.published.url} target="_blank" rel="noreferrer">페이지 열기</a>
                  </p>
                </>
              ) : (
                <>
                  <ol className="crumbs">
                    <li>{st.target?.space_name ?? `스페이스 ${st.space}`}</li>
                    <li>
                      {st.target ? (
                        <a href={st.target.parent_url} target="_blank" rel="noreferrer">{st.target.parent_title}</a>
                      ) : (
                        `상위 페이지 ${st.parent}`
                      )}
                    </li>
                    {byGroup && group && <li className="maybe">{group}</li>}
                    <li className="here new">{name || "(제목)"}</li>
                  </ol>
                  <p className="small">
                    <b>새 페이지를 만듭니다.</b>
                    {byGroup && group && ` 메뉴 그룹 페이지 '${group}'가 없으면 함께 만들고(하위 페이지 목록 표시), 있으면 그 아래에 넣습니다.`}
                  </p>
                  {group && (
                    <label className="cf-check">
                      <input type="checkbox" checked={byGroup} onChange={(e) => setByGroup(e.target.checked)} /> 메뉴 그룹({group}) 페이지 아래에 넣기
                    </label>
                  )}
                </>
              )}
              {!st.target && <p className="muted small">상위 페이지 이름을 아직 모릅니다. "위치 바꾸기"에서 연결 확인을 한 번 하면 이름으로 보입니다.</p>}
            </div>
            <label>
              페이지 제목
              <input value={name} onChange={(e) => setName(e.target.value)} />
            </label>
            <p className="small muted">
              올라갈 내용: 그림 {converted.images.length}장(첨부) · 표 {count(/<table>/g)}개 · 패널 {count(/ac:name="(info|note|tip|warning)"/g)}개 · 상태 라벨 {count(/ac:name="status"/g)}개
              {dirty && " · 저장 안 한 편집 포함"}
            </p>
            {conflict && (
              <div className="flash err">
                같은 제목의 페이지가 이미 있습니다:{" "}
                <a href={conflict} target="_blank" rel="noreferrer">
                  열어 보기
                </a>
                . 그 페이지를 이 매뉴얼로 갱신할까요?{" "}
                <button className="btn small" disabled={busy} onClick={() => publish(true)}>
                  그 페이지 갱신
                </button>{" "}
                또는 위 제목을 바꾸세요.
              </div>
            )}
            <div className="cf-actions">
              <button className="link" onClick={() => setShowXml(!showXml)}>{showXml ? "저장 형식 닫기" : "저장 형식 보기"}</button>
              <span className="spacer" />
              <button className="btn primary" disabled={busy || !name.trim()} onClick={() => publish(false)}>
                {busy ? "게시하는 중…" : st.published ? "다시 게시" : "게시"}
              </button>
            </div>
            {showXml && <pre className="cf-xml">{converted.storage}</pre>}
          </div>
        )}
        {msg && (
          <p className={`flash ${msg.ok ? "ok" : "err"}`}>
            {msg.text}{" "}
            {msg.link && (
              <a href={msg.link} target="_blank" rel="noreferrer">
                페이지 열기
              </a>
            )}
          </p>
        )}
      </div>
    </div>
  );
}
