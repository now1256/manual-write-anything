import { useState } from "react";
import { shotUrl, type Feature, type Permission, type ShotRecord } from "../api";
import ManualView from "./ManualView";

const TABS = ["개요", "화면·API", "권한", "매뉴얼", "스크린샷"] as const;
type Tab = (typeof TABS)[number];

interface Props {
  feature: Feature;
  permissions: Permission[];
  shot: ShotRecord | null;
  shotsTakenAt: number | null;
  manualVersion: number;
  onCommand: (name: string) => void;
  onClose: () => void;
}

export default function FeatureView({ feature, permissions, shot, shotsTakenAt, manualVersion, onCommand, onClose }: Props) {
  const [tab, setTab] = useState<Tab>("개요");
  const apis = feature.screens.flatMap((s) => s.apis);
  const codes = Array.from(new Set([...feature.permissions, ...feature.screens.flatMap((s) => s.ui_permissions)])).sort();
  const perms = codes.map((c) => permissions.find((p) => p.code === c) ?? null);

  return (
    <div className="feature">
      <div className="feature-head">
        <div>
          <div className="crumb">{feature.group ?? feature.section} ›</div>
          <h2>{feature.label}</h2>
        </div>
        <button className="btn ghost" onClick={onClose} title="Esc">닫기</button>
      </div>
      <div className="tabs" role="tablist">
        {TABS.map((t) => (
          <button key={t} role="tab" aria-selected={t === tab} className={`tab ${t === tab ? "on" : ""}`} onClick={() => setTab(t)}>
            {t}
          </button>
        ))}
      </div>
      <div className="tab-body">
        {tab === "개요" && (
          <dl className="facts">
            <dt>경로</dt>
            <dd className="mono">{feature.href}</dd>
            <dt>라이선스 대메뉴</dt>
            <dd>{feature.license_menu}</dd>
            <dt>노출 권한</dt>
            <dd className="mono">{feature.permissions.length ? feature.permissions.join(", ") : "없음 (항상 보임)"}</dd>
            <dt>화면</dt>
            <dd>{feature.screens.length}개</dd>
            <dt>API</dt>
            <dd>
              {apis.length}개 (백엔드 확인 {apis.filter((a) => a.operation).length})
            </dd>
          </dl>
        )}
        {tab === "화면·API" &&
          feature.screens.map((s) => (
            <div key={s.route} className="screen">
              <div className="screen-head">
                <span className="mono">{s.route}</span>
                <span className="muted small mono">{s.file}</span>
              </div>
              <table className="apis">
                <tbody>
                  {s.apis.map((a) => (
                    <tr key={`${a.method} ${a.path}`}>
                      <td className={`method m-${a.method}`}>{a.method}</td>
                      <td className="mono">{a.path}</td>
                      <td className="arrow">→</td>
                      <td className={`mono ${a.operation ? "ok" : "warn"}`} title={a.operation ?? "OpenAPI에 없음"}>
                        {a.backend ?? "백엔드 미확인"}
                      </td>
                    </tr>
                  ))}
                  {!s.apis.length && (
                    <tr>
                      <td className="muted">API 호출 없음</td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          ))}
        {tab === "권한" && (
          <table className="perms">
            <tbody>
              {perms.map((p, i) => (
                <tr key={codes[i]}>
                  <td className="mono">{codes[i]}</td>
                  <td>{p?.label ?? "라벨 없음"}</td>
                  <td>
                    {p?.server.length ? (
                      <span className="badge ok">서버 강제</span>
                    ) : (
                      <span className="badge warn">화면에서만</span>
                    )}
                    {feature.permissions.includes(codes[i]) && <span className="badge">메뉴 노출</span>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {tab === "매뉴얼" && <ManualView featureKey={feature.key} version={manualVersion} onCommand={onCommand} />}
        {tab === "스크린샷" && <ShotPanel shot={shot} takenAt={shotsTakenAt} onCommand={onCommand} />}
      </div>
    </div>
  );
}

function ShotPanel({ shot, takenAt, onCommand }: { shot: ShotRecord | null; takenAt: number | null; onCommand: (n: string) => void }) {
  if (!shot) {
    return (
      <div className="placeholder">
        아직 이 화면을 촬영하지 않았습니다. <button className="btn" onClick={() => onCommand("shot")}>/shot 실행</button>
      </div>
    );
  }
  const when = takenAt ? new Date(takenAt * 1000).toLocaleString("ko-KR") : "-";
  const diff = shot.stable_diff;
  return (
    <div className="shot">
      <div className="shot-meta">
        <span className={`badge ${shot.ok ? "ok" : "warn"}`}>{shot.ok ? "촬영됨" : "실패"}</span>
        <span className="muted small">{when}</span>
        {shot.width && <span className="muted small mono">{shot.width}×{shot.height}</span>}
        {diff != null && (
          <span className={`badge ${diff <= 0.005 ? "ok" : "warn"}`} title="두 번 찍어 비교한 픽셀 차이">
            2회 차이 {(diff * 100).toFixed(2)}%
          </span>
        )}
        {shot.blocked.length > 0 && (
          <span className="badge warn" title={shot.blocked.map((b) => `${b.method} ${b.url}`).join("\n")}>
            쓰기 차단 {shot.blocked.length}
          </span>
        )}
      </div>
      {shot.error && <p className="error-text">{shot.error}</p>}
      {shot.needs_retake && <p className="warn-text">다시 찍기 필요: {shot.needs_retake}</p>}
      {shot.toasts.length > 0 && <p className="muted small">찍힐 때 떠 있던 알림: {shot.toasts.join(" · ")}</p>}
      {shot.file && (
        <a href={shotUrl(shot.file, shot.sha256)} target="_blank" rel="noreferrer">
          <img className="shot-img" src={shotUrl(shot.file, shot.sha256)} alt={`${shot.label} 화면`} />
        </a>
      )}
      {shot.blocked.length > 0 && (
        <p className="muted small">
          이 화면이 보내려던 쓰기 요청을 공유 서버 보호를 위해 막았습니다: {shot.blocked.map((b) => `${b.method} ${b.url}`).join(", ")}
        </p>
      )}
      {shot.console_errors.length > 0 && (
        <details className="small">
          <summary>브라우저 콘솔 오류 {shot.console_errors.length}건</summary>
          <pre className="mono">{shot.console_errors.join("\n")}</pre>
        </details>
      )}
    </div>
  );
}
