import { useEffect, useState } from "react";
import AutomationPanel from "./AutomationPanel";
import { fetchManuals, type ManualStatus, type ProductMap, type ShotsManifest } from "../api";

const pct = (a: number, b: number) => (b ? `${((100 * a) / b).toFixed(1)}%` : "-");

interface Props {
  map: ProductMap | null;
  shots: ShotsManifest | null;
  onCommand: (n: string, arg?: string) => void;
  onOpen: (feature: string) => void;
  /** Bumped after commands finish. */
  version: number;
}

export default function Overview({ map, shots, onCommand, onOpen, version }: Props) {
  if (!map) {
    return (
      <div className="empty">
        <p>기능 목록(제품 지도)이 아직 없습니다. 위 "시작하기"를 순서대로 하면 "코드 읽고 제품 파악" 단계에서 만들어집니다.</p>
      </div>
    );
  }
  const c = map.coverage;
  const g = map.gaps;
  const server = map.permissions.filter((p) => p.server.length).length;
  const tiles: [string, string, string][] = [
    ["메뉴", `${c.menus}`, `화면 연결 ${c.menus_with_screen}`],
    ["화면", `${c.screens}`, `API 있는 화면 ${c.screens_with_api}`],
    ["화면 → 프록시", pct(c.client_calls_linked, c.client_calls), `${c.client_calls_linked}/${c.client_calls}`],
    ["프록시 → 백엔드", pct(c.proxy_resolved, c.proxy_handlers), `${c.proxy_resolved}/${c.proxy_handlers}`],
    ["백엔드 → OpenAPI", pct(c.upstream_with_operation, c.upstream_calls), `${c.upstream_with_operation}/${c.upstream_calls}`],
    ["권한", `${map.permissions.length}`, `서버 강제 ${server}`],
  ];
  if (shots) {
    const ok = shots.shots.filter((s) => s.ok).length;
    const cmp = shots.shots.filter((s) => s.stable_diff != null);
    const stable = cmp.filter((s) => (s.stable_diff ?? 1) <= 0.005).length;
    const retake = shots.shots.filter((s) => s.needs_retake).length;
    tiles.push(["스크린샷", `${ok}/${shots.shots.length}`, `2회 일치 ${stable}/${cmp.length} · 다시 찍기 ${retake}`]);
  }
  return (
    <div className="overview">
      <Manuals onOpen={onOpen} version={version} />
      <AutomationPanel
        features={map.features.map((f) => ({ key: f.key, label: f.label }))}
        version={version}
        onCommand={onCommand}
      />
      <details className="map-details">
        <summary>제품 지도 (코드 분석 결과) — 메뉴 {c.menus} · 화면 {c.screens} · 권한 {map.permissions.length}</summary>
      <div className="overview-head">
        <h2>제품 지도</h2>
        <button className="btn ghost" onClick={() => onCommand("map")}>다시 만들기</button>
      </div>
      <div className="tiles">
        {tiles.map(([k, v, sub]) => (
          <div key={k} className="tile">
            <div className="tile-k">{k}</div>
            <div className="tile-v">{v}</div>
            <div className="tile-sub">{sub}</div>
          </div>
        ))}
      </div>
      <h3>확인 필요</h3>
      <ul className="gaps">
        <li>
          OpenAPI에 없는 백엔드 호출 <b>{g.missing_backend_operations.length}</b>
          <span className="muted"> — 프론트가 부르는데 백엔드에 아직 없는 엔드포인트</span>
        </li>
        <li>백엔드 호출을 알아내지 못한 프록시 <b>{g.unresolved_routes.length}</b></li>
        <li>URL을 풀지 못한 화면 호출 <b>{g.unresolved_client_calls.length}</b></li>
      </ul>
      {map.sources.openapi && (
        <p className="muted small">
          OpenAPI {map.sources.openapi.origin} · {map.sources.openapi.operations}개 오퍼레이션
        </p>
      )}
      </details>
    </div>
  );
}

const day = (t: number) => new Date(t * 1000).toLocaleDateString("ko-KR", { month: "numeric", day: "numeric" });

/** The three jobs, then where every feature's manual stands. */
function Manuals({ onOpen, version }: { onOpen: (f: string) => void; version: number }) {
  const [rows, setRows] = useState<ManualStatus[] | null>(null);
  const [publish, setPublish] = useState(false);
  const [filter, setFilter] = useState<"all" | "todo" | "ref">("all");
  useEffect(() => {
    fetchManuals()
      .then((r) => {
        setRows(r.features);
        setPublish(r.publish);
      })
      .catch(() => setRows([]));
  }, [version]);
  if (!rows) return <p className="muted">매뉴얼 현황을 불러오는 중…</p>;
  const next = (r: ManualStatus): { text: string; warn?: boolean } => {
    if (r.reference && r.reference.described < r.reference.pictures) return { text: "① 장면 설명", warn: true };
    if (r.reference && !r.scenes) return { text: "② 장면 찍기", warn: true };
    if (!r.code) return { text: "③ 코드 읽기", warn: true };
    if (!r.doc) return { text: "③ 매뉴얼 쓰기", warn: true };
    if (r.code.changed) return { text: "③ 코드가 바뀜 — 다시 읽고 제안 받기", warn: true };
    if (!r.grounding || r.grounding.stale) return { text: "④ 근거 확인" };
    if (r.grounding.contradicted) return { text: `④ 코드와 다름 ${r.grounding.contradicted}곳 고치기`, warn: true };
    if (publish && !r.published) return { text: "④ 게시" };
    return { text: "완료" };
  };
  const shown = rows.filter((r) => (filter === "ref" ? !!r.reference : filter === "todo" ? next(r).text !== "완료" : true));
  const done = rows.filter((r) => next(r).text === "완료").length;
  return (
    <section className="manuals">
      <div className="jobs">
        <div className="job">
          <b>기존 매뉴얼 갱신</b>
          <p>① 기존 매뉴얼(PDF·MD) 연결 → ② 지금 화면으로 그림 다시 찍기·고르기 → ③ 코드로 다시 쓰기 → ④ 고치고 저장</p>
        </div>
        <div className="job">
          <b>새 매뉴얼 쓰기</b>
          <p>매뉴얼이 없는 새 기능은 ③에서 코드를 읽고, 이미 만든 매뉴얼을 본보기로 같은 양식으로 씁니다 → ④</p>
        </div>
        <div className="job">
          <b>버전 관리</b>
          <p>저장할 때마다 버전이 남습니다. ④에서 기존 매뉴얼·이전 버전과 나란히 비교, 되돌리기, 문단마다 근거(?)와 바뀐 이유(기록)</p>
        </div>
      </div>
      <div className="manuals-head">
        <h2>매뉴얼 현황</h2>
        <span className="muted small">
          {rows.length}개 기능 · 완료 {done} · 기존 매뉴얼 있음 {rows.filter((r) => r.reference).length}
        </span>
        <span className="spacer" />
        <div className="seg">
          {(
            [
              ["all", "전체"],
              ["todo", "할 일 있음"],
              ["ref", "기존 매뉴얼 있음"],
            ] as const
          ).map(([k, label]) => (
            <button key={k} className={filter === k ? "on" : ""} onClick={() => setFilter(k)}>
              {label}
            </button>
          ))}
        </div>
      </div>
      <div className="manuals-table">
        <table>
          <thead>
            <tr>
              <th>기능</th>
              <th>기존 매뉴얼</th>
              <th>그림</th>
              <th>문서</th>
              <th>근거</th>
              <th>코드</th>
              {publish && <th>게시</th>}
              <th>다음 할 일</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((r) => {
              const n = next(r);
              return (
                <tr key={r.key} onClick={() => onOpen(r.key)} title="열기">
                  <td>
                    <span className="muted small">{r.group} › </span>
                    <b>{r.label}</b>
                  </td>
                  <td>{r.reference ? r.reference.title : <span className="muted">없음 (새로 씀)</span>}</td>
                  <td>
                    {r.scenes ? (
                      <>
                        지금 화면 {r.scenes.reproduced + r.scenes.different} · 못 찍음 {r.scenes.failed}
                        {r.scenes.uploaded ? ` · 올림 ${r.scenes.uploaded}` : ""}
                      </>
                    ) : r.reference ? (
                      <span className="muted">안 찍음</span>
                    ) : (
                      "-"
                    )}
                  </td>
                  <td>{r.doc ? `버전 ${r.doc.version} · ${day(r.doc.updated_at)}` : <span className="muted">없음</span>}</td>
                  <td>
                    {r.grounding ? (
                      <>
                        <span className="ok">✓{r.grounding.supported}</span> <span className="warn">◐{r.grounding.partial}</span>{" "}
                        <span className={r.grounding.contradicted ? "err" : "muted"}>!{r.grounding.contradicted}</span>
                        {r.grounding.stale && <span className="muted small"> (다시 확인)</span>}
                      </>
                    ) : (
                      <span className="muted">-</span>
                    )}
                  </td>
                  <td>
                    {r.code ? (
                      <>
                        사실 {r.code.facts} · {day(r.code.read_at)}
                        {r.code.changed && <span className="warn"> · 바뀜</span>}
                      </>
                    ) : (
                      <span className="muted">안 읽음</span>
                    )}
                  </td>
                  {publish && <td>{r.published ? `v${r.published.version}` : <span className="muted">-</span>}</td>}
                  <td className={n.warn ? "warn" : n.text === "완료" ? "ok" : ""}>{n.text}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}
