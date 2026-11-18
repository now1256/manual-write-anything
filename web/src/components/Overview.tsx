import type { ProductMap, ShotsManifest } from "../api";

const pct = (a: number, b: number) => (b ? `${((100 * a) / b).toFixed(1)}%` : "-");

interface Props {
  map: ProductMap | null;
  shots: ShotsManifest | null;
  onCommand: (n: string) => void;
}

export default function Overview({ map, shots, onCommand }: Props) {
  if (!map) {
    return (
      <div className="empty">
        <p>제품 지도가 아직 없습니다.</p>
        <button className="btn" onClick={() => onCommand("map")}>/map 실행</button>
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
    </div>
  );
}
