import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Channel,
  fetchMap,
  fetchSetup,
  fetchShots,
  fetchState,
  setChatModel,
  type AppState,
  type ProductMap,
  type ServerMsg,
  type ShotsManifest,
} from "./api";
import { COMMANDS } from "./commands";
import ProjectSwitcher from "./components/ProjectSwitcher";
import SettingsView from "./components/SettingsView";
import SetupWizard from "./components/SetupWizard";
import SetupChecklist from "./components/SetupChecklist";
import Sidebar from "./components/Sidebar";
import FeatureView from "./components/FeatureView";
import Overview from "./components/Overview";
import StatusBar from "./components/StatusBar";
import RunBanner, { type Notice, type Run } from "./components/RunBanner";

let nextId = 1;

export default function App() {
  const [state, setState] = useState<AppState | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  /** A setup step opened in the settings: only its section shows. */
  const [settingsFocus, setSettingsFocus] = useState<string | null>(null);
  /** The setup wizard: opens by itself while a required step is not done. */
  const [wizard, setWizard] = useState(false);
  const [setupVersion, setSetupVersion] = useState(0);
  const [map, setMap] = useState<ProductMap | null>(null);
  const [shots, setShots] = useState<ShotsManifest | null>(null);
  const [manualVersion, setManualVersion] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  /** What is running now (shown at the top instead of a terminal). */
  const [run, setRun] = useState<Run | null>(null);
  const [notices, setNotices] = useState<Notice[]>([]);
  const busy = run?.label ?? null;
  const [connected, setConnected] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const channel = useRef<Channel | null>(null);
  const pending = useRef<number | null>(null);

  /** Result or error of a command, shown at the top until closed. */
  const notify = useCallback((ok: boolean, title: string, lines: string[] = []) => {
    setNotices((n) => [{ id: nextId++, ok, title, lines }, ...n].slice(0, 3));
  }, []);

  const reload = useCallback(async () => {
    try {
      const [s, m, sh] = await Promise.all([fetchState(), fetchMap(), fetchShots()]);
      setState(s);
      setMap(m);
      setShots(sh);
      setLoadError(null);
    } catch (e) {
      setLoadError(String(e));
    }
  }, []);

  const onMessage = useCallback(
    (m: ServerMsg) => {
      switch (m.type) {
        case "status":
          setState((s) => (s ? { ...s, status: m.status } : s));
          return;
        case "lines":
          notify(true, m.title, m.lines);
          break;
        case "map_built":
          notify(true, "제품 지도를 다시 만들었습니다", m.lines);
          void reload();
          break;
        case "progress":
          setRun((r) => (r && r.id === m.id ? { ...r, line: m.text } : r));
          return;
        case "manual_done":
          notify(true, "코드를 읽었습니다 (코드 사실)", m.lines);
          setManualVersion((v) => v + 1);
          break;
        case "shots_done":
          notify(true, "촬영을 마쳤습니다", m.lines);
          void fetchShots().then(setShots);
          break;
        case "error":
          notify(false, "실패했습니다", [m.message]);
          break;
        default:
          return;
      }
      if (pending.current === m.id) {
        pending.current = null;
        setRun(null);
        setSetupVersion((v) => v + 1);
      }
    },
    [notify, reload],
  );

  useEffect(() => {
    void reload();
    const ch = new Channel(onMessage, setConnected);
    channel.current = ch;
    return () => ch.close();
  }, [onMessage, reload]);

  // A project opened (or switched to) with a required step left: the wizard first.
  const workspace = state?.header.workspace ?? null;
  useEffect(() => {
    if (!workspace) return;
    fetchSetup()
      .then((steps) => {
        const required = ["code", "claude", "product", "account"];
        if (steps.some((s) => required.includes(s.key) && !s.done)) setWizard(true);
      })
      .catch(() => {});
  }, [workspace]);

  const feature = useMemo(() => map?.features.find((f) => f.key === selected) ?? null, [map, selected]);

  const LABELS: Record<string, string> = {
    map: "제품 지도 생성",
    adapter: "코드 읽고 제품 파악",
    auto: "자동 작업",
    doctor: "점검",
    shot: "촬영",
    gen: "코드 읽기",
    sync: "소스 동기화",
    browser: "브라우저 설치",
    describe: "장면 설명 만들기",
    shotref: "장면 촬영",
    shotdoc: "그림 자리 촬영",
    docref: "기존 매뉴얼 옮기기",
    docupdate: "갱신 제안 받기",
    docmake: "매뉴얼 쓰기",
    docground: "근거 확인",
  };

  const runCommand = (name: string, given?: string) => {
    if (!COMMANDS.some((c) => c.name === name)) return notify(false, `알 수 없는 작업: ${name}`);
    if (busy) return notify(false, `${busy}이(가) 끝난 뒤에 다시 눌러 주세요.`);
    const id = nextId++;
    const arg = given ?? (name === "gen" ? selected ?? undefined : undefined);
    if (!channel.current?.send({ type: "command", id, name, arg })) return notify(false, "서버에 연결되어 있지 않습니다.");
    pending.current = id;
    setRun({ id, label: LABELS[name] ?? name, line: "", started: Date.now() });
  };

  const changeModel = async (model: string) => {
    try {
      const status = await setChatModel(model);
      setState((s) => (s ? { ...s, status } : s));
    } catch (e) {
      notify(false, String(e));
    }
  };

  return (
    <div className="app">
      <header className="topbar">
        <span className="brand">mwa</span>
        <ProjectSwitcher current={state?.header.workspace ?? null} />
        <button className={`topbar-button ${showSettings ? "active" : ""}`} onClick={() => {
            setSettingsFocus(null);
            setWizard(false);
            setShowSettings((v) => !v);
          }}>
          설정
        </button>
        <span className="spacer" />
        {state?.header.sources.map((s) => (
          <span key={s.name} className="source">
            <b>{s.name}</b> <span className="mono">{s.sha.slice(0, 7)}</span>
            {s.preview && <span className="badge warn">미리보기</span>}
          </span>
        ))}
      </header>
      <Sidebar map={map} selected={selected} onSelect={setSelected} />
      <main className="content">
        <RunBanner run={run} notices={notices} onClose={(id) => setNotices((n) => n.filter((x) => x.id !== id))} />
        <section className="detail">
          {loadError ? (
            <div className="empty error">서버에서 정보를 불러오지 못했습니다: {loadError}</div>
          ) : showSettings ? (
            <SettingsView
              focus={settingsFocus}
              onClose={() => {
                setShowSettings(false);
                setSettingsFocus(null);
                setSetupVersion((v) => v + 1);
              }}
              onCommand={runCommand}
            />
          ) : wizard ? (
            <SetupWizard
              version={setupVersion}
              busy={busy}
              onCommand={runCommand}
              onClose={() => {
                setWizard(false);
                setSetupVersion((v) => v + 1);
                void reload();
              }}
            />
          ) : feature ? (
            <FeatureView
              feature={feature}
              permissions={map?.permissions ?? []}
              manualVersion={manualVersion}
              refVersion={setupVersion}
              onCommand={runCommand}
              onClose={() => setSelected(null)}
            />
          ) : (
            <>
              <SetupChecklist version={setupVersion} onOpen={() => setWizard(true)} />
              <Overview map={map} shots={shots} onCommand={runCommand} onOpen={setSelected} version={setupVersion + manualVersion} />
            </>
          )}
        </section>
      </main>
      <StatusBar
        status={state?.status ?? null}
        models={state?.chat_models ?? []}
        busy={busy}
        connected={connected}
        onModel={(m) => void changeModel(m)}
      />
    </div>
  );
}
