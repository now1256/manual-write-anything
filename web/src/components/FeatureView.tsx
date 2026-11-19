import { useEffect, useState } from "react";
import { fetchReferences, type Feature, type Permission } from "../api";
import DocEditor from "./DocEditor";
import FactsView from "./FactsView";
import MakeStep from "./MakeStep";
import ReferenceView from "./ReferenceView";
import SourceStep from "./SourceStep";

/** The four steps of making a feature's manual, then reference tabs. */
const STEPS = ["① 기존 매뉴얼", "② 그림 고르기", "③ 매뉴얼 만들기", "④ 편집·버전"] as const;
const TABS = [...STEPS, "개요", "화면·API", "권한", "코드 사실"] as const;
type Tab = (typeof TABS)[number];

interface Props {
  feature: Feature;
  permissions: Permission[];
  manualVersion: number;
  /** Bumped after commands finish (e.g. scene descriptions). */
  refVersion: number;
  onCommand: (name: string, arg?: string) => void;
  onClose: () => void;
}

export default function FeatureView({ feature, permissions, manualVersion, refVersion, onCommand, onClose }: Props) {
  const [tab, setTab] = useState<Tab>("① 기존 매뉴얼");
  // A feature without an old manual starts at ③ (write from code).
  /** An old manual is paired: without one there are no pictures to choose (②). */
  const [paired, setPaired] = useState(true);
  /** Bumped when ① links or unlinks an old manual. */
  const [linked, setLinked] = useState(0);
  // Opening a feature without an old manual starts at ③.
  useEffect(() => {
    let live = true;
    fetchReferences()
      .then((l) => {
        if (live && !l.items.some((d) => d.feature === feature.key && !d.ignored)) setTab("③ 매뉴얼 만들기");
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [feature.key]);
  useEffect(() => {
    let live = true;
    fetchReferences()
      .then((l) => live && setPaired(l.items.some((d) => d.feature === feature.key && !d.ignored)))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [feature.key, refVersion, linked]);
  /** Picture to scroll to after switching tabs; `n` re-triggers the same picture. */
  const [focus, setFocus] = useState<{ image: string; n: number } | null>(null);
  const goto = (to: Tab, image: string) => {
    setTab(to);
    setFocus((f) => ({ image, n: (f?.n ?? 0) + 1 }));
  };
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
        {TABS.map((t, i) => (
          <button
            key={t}
            role="tab"
            aria-selected={t === tab}
            className={`tab ${t === tab ? "on" : ""} ${i < STEPS.length ? "step-tab" : ""} ${i === STEPS.length ? "tab-gap" : ""}`}
            disabled={t === "② 그림 고르기" && !paired}
            title={t === "② 그림 고르기" && !paired ? "기존 매뉴얼이 연결되어 있지 않습니다. 그림은 ④의 그림 자리로 찍습니다" : undefined}
            onClick={() => setTab(t)}
          >
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
        {/* Kept mounted so unsaved edits survive tab switches. */}
        {tab === "① 기존 매뉴얼" && (
          <SourceStep featureKey={feature.key} version={refVersion} onCommand={onCommand} onNext={() => setTab("② 그림 고르기")} onLinked={() => setLinked((n) => n + 1)} />
        )}
        {tab === "② 그림 고르기" && (
          <ReferenceView
            featureKey={feature.key}
            version={refVersion}
            onCommand={onCommand}
            focus={focus}
            onGotoManual={(image) => goto("④ 편집·버전", image)}
          />
        )}
        {tab === "③ 매뉴얼 만들기" && (
          <MakeStep
            featureKey={feature.key}
            version={refVersion + manualVersion}
            onCommand={onCommand}
            onEdit={() => setTab("④ 편집·버전")}
            onPictures={() => setTab("② 그림 고르기")}
          />
        )}
        <div hidden={tab !== "④ 편집·버전"}>
          <DocEditor
            featureKey={feature.key}
            version={refVersion + manualVersion}
            focus={tab === "④ 편집·버전" ? focus : null}
            onGotoReference={(image) => goto("② 그림 고르기", image)}
            onGoMake={() => setTab("③ 매뉴얼 만들기")}
            active={tab === "④ 편집·버전"}
            onCommand={onCommand}
            group={feature.group ?? feature.section ?? null}
          />
        </div>
        {tab === "코드 사실" && <FactsView featureKey={feature.key} version={manualVersion} onCommand={onCommand} />}
      </div>
    </div>
  );
}
