import { useEffect, useState } from "react";
import {
  fetchDoc,
  fetchManual,
  fetchProposals,
  fetchReference,
  fetchReferences,
  fetchSceneShots,
  fetchState,
  fetchVersions,
  type ManualDoc,
  type ManualRun,
  type RefDoc,
  type SceneShots,
} from "../api";
import { pickPicture } from "../pictures";

interface Props {
  featureKey: string;
  version: number;
  onCommand: (name: string, arg?: string) => void;
  onEdit: () => void;
  onPictures: () => void;
}

/** Code commits the draft was read from vs. the checked-out ones now. */
interface CodeAge {
  then: { name: string; sha: string }[] | null;
  now: { name: string; sha: string }[];
}

/** Step ③: make the document once, then keep it in step with the product's code. */
export default function MakeStep({ featureKey, version, onCommand, onEdit, onPictures }: Props) {
  const [ref, setRef] = useState<RefDoc | null | undefined>(undefined);
  const [shots, setShots] = useState<SceneShots | null>(null);
  const [run, setRun] = useState<ManualRun | null>(null);
  const [doc, setDoc] = useState<ManualDoc | null>(null);
  const [pending, setPending] = useState(0);
  const [age, setAge] = useState<CodeAge | null>(null);

  useEffect(() => {
    let live = true;
    fetchReferences()
      .then((l) => l.items.find((d) => d.feature === featureKey && !d.ignored))
      .then((s) => (s ? fetchReference(s.id) : null))
      .then((d) => live && setRef(d))
      .catch(() => live && setRef(null));
    fetchSceneShots(featureKey).then((s) => live && setShots(s)).catch(() => live && setShots(null));
    fetchManual(featureKey).then((r) => live && setRun(r)).catch(() => live && setRun(null));
    fetchDoc(featureKey).then((d) => live && setDoc(d)).catch(() => live && setDoc(null));
    fetchProposals(featureKey).then((p) => live && setPending(p.items.length)).catch(() => {});
    Promise.all([fetchVersions(featureKey).catch(() => []), fetchState()])
      .then(([v, st]) => {
        const last = v[v.length - 1]?.meta;
        live && setAge({ then: last ? last.sources : null, now: (st?.header.sources ?? []).map((s) => ({ name: s.name, sha: s.sha })) });
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [featureKey, version]);

  if (ref === undefined) return <p className="muted">불러오는 중…</p>;

  const count: Record<string, number> = {};
  for (const img of ref?.images ?? []) {
    const s = shots?.scenes.find((x) => x.image === img.file);
    const p = pickPicture(s, !!img.scene && !img.scene.capture);
    const k = p.kind === "old" ? "old" : p.kind === "chosen" ? "chosen" : p.source;
    count[k] = (count[k] ?? 0) + 1;
  }
  const changed = age?.then
    ? age.now.filter((n) => {
        const t = age.then!.find((x) => x.name === n.name);
        // Shas may be stored at different lengths.
        return !t || !(t.sha.startsWith(n.sha) || n.sha.startsWith(t.sha));
      })
    : [];
  const short = (sha: string) => sha.slice(0, 10);


  return (
    <div className="step make">
      <h3>③ 매뉴얼 만들기</h3>

      <section className={`make-card ${doc ? "done" : "now"}`}>
        <div className="make-title">
          <span className="make-no">A</span>
          <div>
            <b>코드로 매뉴얼 쓰기</b>
            <p className="muted small">
              내용은 지금 코드에서 뽑은 사실로 쓰고, {ref ? "목차·제목·표·말투는 기존 매뉴얼을 따릅니다. 바뀐 곳은 고치고 새 기능은 추가하며, 코드로 확인할 수 없는 운영 안내는 기존 글을 남깁니다." : "양식은 이미 만든 다른 기능의 매뉴얼을 본보기로 따르고, 그림이 들어갈 곳마다 찍을 장면을 정해 지금 화면으로 바로 찍습니다."}{" "}
              문단마다 왜 그렇게 썼는지(근거)가 함께 남습니다.
            </p>
          </div>
          <span className="spacer" />
          <button className="btn primary" disabled={!run} title={run ? "" : "아래 1번 '코드 읽기'를 먼저 하세요"} onClick={() => onCommand("docmake", featureKey)}>
            {doc ? "다시 쓰기" : "매뉴얼 쓰기"}
          </button>
        </div>
        <ul className="make-inputs">
          <li className={run ? "ok" : "warn"}>
            <b>내용</b>{" "}
            {run ? (
              `코드 사실 ${run.facts.length}개 (${new Date(run.generated_at * 1000).toLocaleString("ko-KR")} 기준)`
            ) : (
              <>
                코드를 아직 읽지 않았습니다 — <button className="link" onClick={() => onCommand("gen", featureKey)}>코드 읽기</button> (몇 분)
              </>
            )}
          </li>
          <li className="ok">
            <b>양식</b> {ref ? `${ref.title} · ${ref.pages ? `${ref.pages}쪽 PDF` : "MD"} (기존 매뉴얼, 버전 0으로 보관해 비교)` : "다른 기능의 매뉴얼을 본보기로"}
          </li>
          {ref && (
            <li className={count.old ? "warn" : "ok"}>
              <b>그림</b> 지금 화면 {count.current ?? 0} · 올린 그림 {count.upload ?? 0} · 기존 그림(고름) {count.chosen ?? 0} · 예전 화면 그대로 {count.old ?? 0}
              <button className="link" onClick={onPictures}>② 그림 고르기에서 바꾸기</button>
              <div className="muted small">지금 화면 그림은 매뉴얼을 쓸 때 모델이 직접 봅니다. 화면에 새로 생긴 컬럼·버튼·입력 칸과 ②에서 찾은 "화면이 바뀐 점"이 글에 들어갑니다.</div>
            </li>
          )}
          {!ref && (
            <li className="ok">
              <b>그림</b> 쓰고 나면 그림 자리를 바로 찍습니다 (개발 서버에 로그인 1회, 쓰기 요청은 막힘). 못 찍은 자리는 ④에서 다시 찍거나 올립니다.
            </li>
          )}
        </ul>
        {doc && <p className="muted small">지금 문서(버전 {doc.version})는 다시 써도 버전으로 남습니다. 사람이 고친 내용은 다시 써지므로, 코드가 조금 바뀐 정도면 아래 B의 "제안 받기"가 낫습니다.</p>}
        {ref && (
          <details className="step-change">
            <summary>기존 매뉴얼 없이 처음부터 새로 만들기</summary>
            <p className="muted small">
              연결한 기존 매뉴얼({ref.title})을 쓰지 않고, 다른 기능의 매뉴얼을 본보기로 코드로 새로 쓴 뒤 그림 자리를 지금 화면으로 찍습니다. 기존 매뉴얼의 목차와 그림을 버리고 싶을 때 씁니다. 지금 문서는 버전으로 남습니다. 앞으로도 계속 이렇게 하려면 ①에서 연결을 해제하세요.
            </p>
            <button className="btn" disabled={!run} onClick={() => onCommand("docmake", `${featureKey}?fresh`)}>
              처음부터 새로 만들기
            </button>
          </details>
        )}
        <details className="step-change">
          <summary>기존 매뉴얼 글을 그대로 옮기기 (예비)</summary>
          <p className="muted small">코드로 다시 쓰지 않고 기존 매뉴얼의 글을 그대로 문서로 옮깁니다. 코드와 다른 곳은 B의 제안으로 고칩니다.</p>
          <button className="btn" disabled={!ref} onClick={() => onCommand("docref", featureKey)}>그대로 옮기기</button>
        </details>
      </section>

      <section className={`make-card ${doc ? "now" : "later"}`}>
        <div className="make-title">
          <span className="make-no">B</span>
          <div>
            <b>제품이 바뀌었을 때 — 코드 기준 확인</b>
            <p className="muted small">
              개발자가 기능을 고치거나 추가해 코드가 바뀌면, 문서에서 달라진 곳을 찾아 고칠 제안을 받습니다. 문서는 바로 바뀌지 않고 ④에서 수락·거절합니다.
            </p>
          </div>
        </div>
        <div className={`code-age ${changed.length ? "warn" : ""}`}>
          {!age ? (
            "코드 상태를 확인하는 중…"
          ) : !run ? (
            "아직 코드를 읽은 적이 없습니다. 1번부터 하세요."
          ) : !age.then ? (
            "코드를 읽은 때의 커밋 기록이 없습니다 (버전 기록 이전에 만듦). 코드가 바뀌었는지 모르면 1번부터 하세요."
          ) : changed.length ? (
            <>
              ● 코드가 바뀌었습니다:{" "}
              {changed.map((n) => {
                const t = age.then!.find((x) => x.name === n.name);
                return (
                  <span key={n.name} className="mono">
                    {n.name} {t ? short(t.sha) : "?"} → {short(n.sha)}{" "}
                  </span>
                );
              })}
              — 1번부터 하세요.
            </>
          ) : (
            <>✓ 마지막으로 읽은 코드가 지금 코드와 같습니다 ({age.now.map((n) => `${n.name} ${short(n.sha)}`).join(", ")}). 2번만 하면 됩니다.</>
          )}
        </div>
        <ol className="make-steps">
          <li>
            <div>
              <b>코드 다시 읽기</b>
              <p className="muted small">
                Git의 지금 코드를 처음부터 읽어 이 기능의 사실(버튼·필드·규칙·메시지)을 다시 뽑습니다. 코드가 바뀌었을 때만 필요합니다. 몇 분 걸리고 사용량이 큽니다.
                {run && ` 마지막: ${new Date(run.generated_at * 1000).toLocaleString("ko-KR")} · 사실 ${run.facts.length}개`}
              </p>
            </div>
            <button className={`btn ${changed.length || !run ? "primary" : ""}`} onClick={() => onCommand("gen", featureKey)}>
              코드 다시 읽기
            </button>
          </li>
          <li>
            <div>
              <b>바뀐 곳 제안 받기</b>
              <p className="muted small">지금 문서(사람이 고친 내용 포함)를 1번에서 뽑은 사실과 비교해, 다르거나 빠진 곳만 제안합니다. 1분 정도, 사용량 작음.</p>
            </div>
            <button className="btn primary" disabled={!doc || !run} title={!doc ? "먼저 A에서 만드세요" : !run ? "먼저 1번을 하세요" : ""} onClick={() => onCommand("docupdate", featureKey)}>
              제안 받기
            </button>
          </li>
          <li>
            <div>
              <b>수락·거절하고 저장</b>
              <p className="muted small">④ 편집·버전 위쪽에 제안이 뜹니다. 저장하면 새 버전이 되고 이전 버전과 나란히 비교할 수 있습니다.</p>
            </div>
            <button className="btn" disabled={!doc} onClick={onEdit}>
              ④로 가기{pending ? ` (제안 ${pending}개 대기)` : ""}
            </button>
          </li>
        </ol>
      </section>
    </div>
  );
}
