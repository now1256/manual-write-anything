import { useEffect, useMemo, useState } from "react";
import { pickPicture, type Picked } from "../pictures";
import {
  fetchReference,
  fetchReferences,
  fetchSceneShots,
  refFileUrl,
  refPageUrl,
  docAssetUrl,
  setSceneUpload,
  setSceneChoice,
  setSceneInstruction,
  shotUrl,
  type RefDoc,
  type RefImage,
  type SceneShot,
  type SceneShots,
} from "../api";

const STATUS: Record<string, { mark: string; label: string }> = {
  reproduced: { mark: "✓", label: "재현됨" },
  different: { mark: "△", label: "다름" },
  failed: { mark: "✗", label: "못 찍음" },
  not_taken: { mark: "–", label: "아직 안 찍음" },
};

const REASON: Record<string, string> = {
  needs_write: "실제 서버 쓰기 필요",
  realtime: "실시간·외부 의존",
  account: "다른 계정·권한 필요",
  not_found: "화면·버튼을 못 찾음",
  no_data: "그 상태의 데이터 없음",
  wrong_screen: "다른 화면이 찍힘",
  error: "오류",
};

interface Props {
  featureKey: string;
  version: number;
  onCommand: (name: string, arg?: string) => void;
  /** Picture to scroll to (from the new manual). */
  focus: { image: string; n: number } | null;
  onGotoManual: (image: string) => void;
}

const KIND: Record<string, string> = {
  screen: "화면",
  dialog: "대화상자",
  drawer: "드로어",
  flow: "흐름도",
  diagram: "그림",
  icon: "아이콘",
  other: "기타",
};

/** The old manual (PDF) matched to this feature: pictures grouped by heading, with scenes. */
export default function ReferenceView({ featureKey, version, onCommand, focus, onGotoManual }: Props) {
  const [doc, setDoc] = useState<RefDoc | null | undefined>(undefined);
  const [big, setBig] = useState<{ img: RefImage; page: boolean } | null>(null);
  const [shots, setShots] = useState<SceneShots | null>(null);
  const [demo, setDemo] = useState(false);
  const [err, setErr] = useState("");
  /** Choices and uploads save at once; this tells the user it happened. */
  const [saved, setSaved] = useState<string | null>(null);
  const done = (what: string) => setSaved(`${what} · ${new Date().toLocaleTimeString("ko-KR")}`);
  const shoot = (image?: string) =>
    onCommand("shotref", `${featureKey}${image ? `|${image}` : ""}${demo ? "?demo" : ""}`);
  const instruct = async (image: string, text: string) => {
    try {
      setErr("");
      setShots(await setSceneInstruction(featureKey, image, text.trim() || null));
      done(text.trim() ? "촬영 지시를 저장했습니다" : "촬영 지시를 지웠습니다");
    } catch (x) {
      setErr(String((x as Error).message ?? x));
    }
  };
  const choose = async (image: string, choice: string | null) => {
    try {
      setErr("");
      setShots(await setSceneChoice(featureKey, image, choice));
      done("그림 선택 저장됨");
    } catch (x) {
      setErr(String((x as Error).message ?? x));
    }
  };
  const upload = async (image: string, file: File | null) => {
    try {
      setErr("");
      setShots(await setSceneUpload(featureKey, image, file));
      done(file ? "올린 그림 저장됨" : "올린 그림 뺌");
    } catch (x) {
      setErr(String((x as Error).message ?? x));
    }
  };

  useEffect(() => {
    fetchSceneShots(featureKey).then(setShots).catch(() => setShots(null));
  }, [featureKey, version]);
  const shotOf = (img: RefImage): SceneShot | undefined => shots?.scenes.find((s) => s.image === img.file);

  // Scroll to the picture asked for from the new manual once the cards are there.
  useEffect(() => {
    if (!focus || !doc) return;
    const el = document.querySelector(`.ref-card[data-image="${CSS.escape(focus.image)}"]`);
    if (!el) return;
    el.scrollIntoView({ behavior: "smooth", block: "center" });
    el.classList.add("flash-focus");
    const t = setTimeout(() => el.classList.remove("flash-focus"), 1800);
    return () => clearTimeout(t);
  }, [focus, doc]);

  useEffect(() => {
    let alive = true;
    fetchReferences()
      .then((l) => l.items.find((d) => d.feature === featureKey))
      .then((s) => (s ? fetchReference(s.id) : null))
      .then((d) => alive && setDoc(d))
      .catch(() => alive && setDoc(null));
    return () => {
      alive = false;
    };
  }, [featureKey, version]);

  const groups = useMemo(() => {
    const out: { heading: string; items: RefImage[] }[] = [];
    for (const img of doc?.images ?? []) {
      const last = out[out.length - 1];
      if (last && last.heading === img.heading) last.items.push(img);
      else out.push({ heading: img.heading || "(제목 없음)", items: [img] });
    }
    return out;
  }, [doc]);

  if (doc === undefined) return <p className="muted pad">불러오는 중…</p>;
  if (!doc)
    return (
      <div className="empty">
        <p>이 기능과 연결된 기존 매뉴얼이 없습니다.</p>
        <p className="muted">① 기존 매뉴얼에서 PDF를 연결하세요. 기존 매뉴얼이 없는 기능은 ③에서 코드로 새로 씁니다.</p>
      </div>
    );

  const described = doc.images.filter((i) => i.scene).length;
  const toCapture = doc.images.filter(shootable).length;
  const n = (st: string) => shots?.scenes.filter((s) => s.status === st).length ?? 0;

  return (
    <div className="reference">
      <div className="ref-head">
        <div>
          <h3>② 그림 고르기</h3>
          <span className="muted small">
            기존 그림과 지금 화면을 나란히 보고, 새 매뉴얼에 쓸 그림을 고릅니다. 고르지 않으면 자동(올린 그림 → 찍힌 화면 → 예전 그림)입니다.
          </span>
        </div>
        <span className="spacer" />
        <span className="muted small">
          {described < doc.images.length ? "① 에서 장면 설명을 먼저 만드세요" : `찍을 장면 ${toCapture}장`}
        </span>
        <label className="small" title="끄면 개발 서버의 실제 데이터로 찍습니다 (쓰기 요청은 항상 막음)">
          <input type="checkbox" checked={demo} onChange={(e) => setDemo(e.target.checked)} /> 데모 데이터로
        </label>
        <button
          className="btn"
          disabled={described < doc.images.length || toCapture === 0}
          title={described < doc.images.length ? "장면 설명을 먼저 만드세요" : "장면을 다시 찍고 기존 캡처와 대조합니다"}
          onClick={() => shoot()}
        >
          {shots ? "장면 전체 다시 찍기" : "기존 매뉴얼 장면 찍기"}
        </button>
      </div>
      {shots && (
        <div className="ref-summary">
          <b>장면 촬영 결과</b> {shots.scenes.length}장 중{" "}
          <span className="badge ok">✓ 재현 {n("reproduced")}</span>
          <span className="badge warn">△ 다름 {n("different")}</span>
          <span className="badge err">✗ 못 찍음 {n("failed")}</span>
          {n("not_taken") > 0 && <span className="badge">올리기만 {n("not_taken")}</span>}
          <span className="muted small"> · {shots.taken_at ? new Date(shots.taken_at * 1000).toLocaleString("ko-KR") : "아직 안 찍음"}</span>
          <span className="muted small"> · 못 찍은 장면은 그림을 직접 올리면 새 매뉴얼에 들어갑니다</span>
        </div>
      )}
      {err && <p className="flash err">{err}</p>}
      <div className="autosave">
        {saved ? <span className="ok">✓ {saved}</span> : <span className="muted">고르거나 올리면 바로 저장됩니다 (저장 버튼 없음)</span>}
      </div>
      <nav className="ref-toc">
        {groups.map((g, i) => (
          <a key={i} href={`#ref-${i}`}>{g.heading}</a>
        ))}
      </nav>
      {groups.map((g, gi) => (
        <section key={gi} id={`ref-${gi}`} className="ref-group">
          <h3>{g.heading}</h3>
          {g.items.map((img) => (
            <div key={img.file} data-image={img.file} className={`ref-card ${img.scene && !shootable(img) ? "skip" : ""}`}>
              {shootable(img) ? (
                <div className="ref-pair">
                  <figure>
                    <figcaption>기존 매뉴얼</figcaption>
                    <button className="ref-thumb" onClick={() => setBig({ img, page: false })} title="크게 보기">
                      <img src={refFileUrl(doc.id, img.file)} alt={img.caption || img.heading} loading="lazy" />
                    </button>
                  </figure>
                  <figure>
                    <figcaption>
                      {shotOf(img)?.uploaded ? "올린 그림" : "지금 화면"}
                      <SceneActions
                        shot={shotOf(img)}
                        onShoot={() => shoot(img.file)}
                        onUpload={(f) => upload(img.file, f)}
                      />
                    </figcaption>
                    {shotOf(img)?.uploaded ? (
                      <img src={docAssetUrl(featureKey, shotOf(img)!.uploaded!)} alt="올린 그림" loading="lazy" />
                    ) : shotOf(img)?.file ? (
                      <a href={shotUrl(shotOf(img)!.file!, null)} target="_blank" rel="noreferrer">
                        <img src={shotUrl(shotOf(img)!.file!, String(shots?.taken_at ?? 0))} alt="새 캡처" loading="lazy" />
                      </a>
                    ) : (
                      <div className="none">{shotOf(img) ? "못 찍음" : "아직 안 찍음"}</div>
                    )}
                  </figure>
                  <InstructionBox
                    value={shotOf(img)?.instruction ?? ""}
                    onSave={(t) => instruct(img.file, t)}
                    onShoot={() => shoot(img.file)}
                  />
                </div>
              ) : (
                <div>
                  <button className="ref-thumb" onClick={() => setBig({ img, page: false })} title="크게 보기">
                    <img
                      src={shotOf(img)?.uploaded ? docAssetUrl(featureKey, shotOf(img)!.uploaded!) : refFileUrl(doc.id, img.file)}
                      alt={img.caption || img.heading}
                      loading="lazy"
                    />
                  </button>
                  {img.scene && (
                    <div className="small">
                      {shotOf(img)?.uploaded && <span className="muted">올린 그림이 새 매뉴얼에 들어갑니다 </span>}
                      <SceneActions shot={shotOf(img)} onUpload={(f) => upload(img.file, f)} />
                    </div>
                  )}
                </div>
              )}
              <div className="ref-info">
                {img.scene && (
                  <ChoicePicker
                    shot={shotOf(img)}
                    picked={pickPicture(shotOf(img), !img.scene.capture)}
                    onChoose={(c) => choose(img.file, c)}
                  />
                )}
                <div className="muted small">
                  p.{img.page}{" "}
                  <button className="link" onClick={() => setBig({ img, page: true })}>원래 쪽 보기</button>{" "}
                  <button className="link" onClick={() => onGotoManual(img.file)}>새 매뉴얼의 같은 그림으로 →</button>
                </div>
                {img.context && <p><span className="muted">앞 설명</span> {img.context}</p>}
                {img.caption && <p><span className="muted">캡션</span> {img.caption}</p>}
                {img.scene ? (
                  <div className="scene">
                    <span className={`badge ${img.scene.capture ? "ok" : ""}`}>
                      {KIND[img.scene.kind] ?? img.scene.kind}
                      {img.scene.capture ? " · 찍을 장면" : shootable(img) ? " · 화면 일부 (같은 영역을 잘라 찍음)" : " · 찍지 않음 (그림 올리기만)"}
                    </span>
                    <p><b>{img.scene.screen}</b>{img.scene.state && ` — ${img.scene.state}`}</p>
                    {img.scene.how && <p className="mono small">{img.scene.how}</p>}
                  </div>
                ) : (
                  <p className="muted small">장면 설명 전</p>
                )}
                {(() => {
                  const s = shotOf(img);
                  if (!s) return null;
                  const st = STATUS[s.status] ?? STATUS.failed;
                  return (
                    <div className={`verdict ${s.status}`}>
                      <b>{st.mark} {st.label}</b>
                      {s.status === "failed" && (
                        <span> — {REASON[s.reason_kind ?? "error"] ?? "오류"}{s.reason ? `: ${s.reason}` : ""}</span>
                      )}
                      {!!s.changes?.length && (
                        <div className="screen-changes">
                          <b>화면이 바뀐 점</b> <span className="muted small">③ 매뉴얼 쓰기가 글에 반영합니다</span>
                          <ul>
                            {s.changes.map((d, i) => <li key={i}>{d}</li>)}
                          </ul>
                        </div>
                      )}
                      {s.differences.length > 0 && (
                        <ul>
                          {s.differences.filter((d) => !s.changes?.includes(d)).map((d, i) => <li key={i}>{d}</li>)}
                        </ul>
                      )}
                      {s.status !== "reproduced" && <Trace s={s} takenAt={shots!.taken_at} />}
                    </div>
                  );
                })()}
              </div>
            </div>
          ))}
        </section>
      ))}
      {big && (
        <div className="picker-backdrop" onMouseDown={() => setBig(null)}>
          <div className="ref-big" onMouseDown={(e) => e.stopPropagation()}>
            <div className="ref-big-head">
              <b>{big.img.heading}</b>
              <span className="muted"> · p.{big.img.page}</span>
              <span className="spacer" />
              <button className="btn" onClick={() => setBig({ ...big, page: !big.page })}>
                {big.page ? "캡처만 보기" : "원래 쪽 보기"}
              </button>
              <button className="btn ghost" onClick={() => setBig(null)}>닫기</button>
            </div>
            <img src={big.page ? refPageUrl(doc.id, big.img.page) : refFileUrl(doc.id, big.img.file)} alt="" />
          </div>
        </div>
      )}
    </div>
  );
}

/** "Where it got stuck": every attempt step by step, the screen at that moment, stopped requests. */
function Trace({ s, takenAt }: { s: SceneShot; takenAt: number }) {
  const attempts = s.attempts ?? [];
  const blocked = s.blocked ?? [];
  if (attempts.length === 0 && blocked.length === 0) return null;
  return (
    <details className="trace">
      <summary>어디서 안 됐나</summary>
      {attempts.length === 0 && <p className="muted small">계획 단계에서 찍을 수 없다고 판단해 실행하지 않았습니다.</p>}
      {attempts.map((a, i) => (
        <div key={i} className="trace-try">
          <p className="small">
            <b>{i === 0 ? "처음 계획" : `고친 계획 ${i}`}</b>
            {a.error ? ` — ✗ ${a.error}` : " — ✓ 끝까지 실행"}
          </p>
          <ol>
            {a.steps.map((st, j) => (
              <li key={j} className={st.ok === true ? "ok" : st.ok === false ? "bad" : "skip"}>
                <span className="mark">{st.ok === true ? "✓" : st.ok === false ? "✗" : "–"}</span> {st.text}
                {st.ok !== null && <span className="muted"> ({(st.ms / 1000).toFixed(1)}초)</span>}
                {st.error && <div className="trace-err">{st.error}</div>}
                {st.ok === null && <span className="muted"> (실행 안 됨)</span>}
              </li>
            ))}
          </ol>
          {a.fail_file && (
            <a href={shotUrl(a.fail_file, null)} target="_blank" rel="noreferrer">
              <img className="trace-shot" src={shotUrl(a.fail_file, String(takenAt))} alt="막힌 순간의 화면" loading="lazy" />
            </a>
          )}
        </div>
      ))}
      {blocked.length > 0 && (
        <p className="small">막은 쓰기 요청(공유 서버로 보내지 않음): {blocked.map((b) => <code key={b}>{b} </code>)}</p>
      )}
      {(s.mocked ?? []).length > 0 && <p className="small muted">데모 응답으로 처리: {s.mocked!.join(", ")}</p>}
    </details>
  );
}

/** Screens (whole or cut) can be taken again; drawn flows, diagrams and icons cannot. */
const shootable = (img: RefImage) =>
  !!img.scene && (img.scene.capture || !["flow", "diagram", "icon"].includes(img.scene.kind));

/**
 * A person's instruction for one scene (D-053): the planner follows it before the general rules.
 * Writes stay blocked whatever it says.
 */
function InstructionBox({ value, onSave, onShoot }: { value: string; onSave: (t: string) => Promise<void>; onShoot: () => void }) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const changed = text.trim() !== value;
  return (
    <div className="scene-instr">
      <input
        placeholder="촬영 지시 (선택) — 예: 비활성 상태인 행으로 열어 줘 / 연결 정보 부분만"
        value={text}
        onChange={(e) => setText(e.target.value)}
        title="이 장면을 찍을 때 다른 규칙보다 먼저 따릅니다. 저장·삭제처럼 실제로 바꾸는 동작은 지시해도 막힙니다"
      />
      {changed && (
        <button className="btn small" onClick={() => void onSave(text)}>
          저장
        </button>
      )}
      <button
        className="btn small primary"
        disabled={!text.trim()}
        onClick={async () => {
          if (changed) await onSave(text);
          onShoot();
        }}
      >
        지시대로 다시 찍기
      </button>
      {value && (
        <button className="link small" onClick={() => void onSave("")}>
          지우기
        </button>
      )}
    </div>
  );
}

/** Per-scene fixes: shoot this scene again, or upload a picture in its place. */
function SceneActions({ shot, onShoot, onUpload }: { shot?: SceneShot; onShoot?: () => void; onUpload: (f: File | null) => void }) {
  return (
    <span className="scene-actions">
      {onShoot && <button className="btn small" onClick={onShoot} title="이 장면만 다시 찍습니다 (다른 장면 결과는 그대로)">다시 찍기</button>}
      <label className="btn small" title="찍기 어려운 장면은 그림을 직접 올립니다">
        그림 올리기
        <input
          type="file"
          accept="image/png,image/jpeg,image/gif,image/webp"
          hidden
          onChange={(e) => {
            const f = e.target.files?.[0];
            e.target.value = "";
            if (f) onUpload(f);
          }}
        />
      </label>
      {shot?.uploaded && <button className="btn small" onClick={() => onUpload(null)}>올린 그림 빼기</button>}
    </span>
  );
}

/** Which picture goes into the new manual for this scene. */
function ChoicePicker({ shot, picked, onChoose }: { shot?: SceneShot; picked: Picked; onChoose: (c: string | null) => void }) {
  const options: { value: string | null; label: string; disabled?: boolean; title?: string }[] = [
    { value: null, label: "자동" },
    { value: "current", label: "지금 화면", disabled: !shot?.file, title: shot?.file ? "" : "찍힌 화면이 없습니다" },
    { value: "old", label: "기존 그림" },
    { value: "upload", label: "올린 그림", disabled: !shot?.uploaded, title: shot?.uploaded ? "" : "그림을 먼저 올리세요" },
  ];
  const current = shot?.choice ?? null;
  return (
    <div className="choice">
      <span className="muted small">새 매뉴얼에 쓸 그림</span>
      <div className="seg">
        {options.map((o) => (
          <button
            key={o.label}
            type="button"
            className={current === o.value ? "on" : ""}
            disabled={o.disabled}
            title={o.title}
            onClick={() => onChoose(o.value)}
          >
            {o.label}
          </button>
        ))}
      </div>
      <span className={`fig-badge ${picked.kind}`}>→ {picked.label}</span>
    </div>
  );
}
