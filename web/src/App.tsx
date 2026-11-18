import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Channel,
  fetchMap,
  fetchShots,
  fetchState,
  setChatModel,
  type AppState,
  type ProductMap,
  type ServerMsg,
  type ShotsManifest,
  type Who,
} from "./api";
import { HELP_LINES, COMMANDS } from "./commands";
import Sidebar from "./components/Sidebar";
import FeatureView from "./components/FeatureView";
import Overview from "./components/Overview";
import Chat, { type ChatMsg } from "./components/Chat";
import StatusBar from "./components/StatusBar";

let nextId = 1;

export default function App() {
  const [state, setState] = useState<AppState | null>(null);
  const [map, setMap] = useState<ProductMap | null>(null);
  const [shots, setShots] = useState<ShotsManifest | null>(null);
  const [manualVersion, setManualVersion] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const [msgs, setMsgs] = useState<ChatMsg[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [chatOpen, setChatOpen] = useState(true);
  const channel = useRef<Channel | null>(null);
  const pending = useRef<number | null>(null);

  const push = useCallback((who: Who, text: string) => {
    setMsgs((m) => [...m, { id: nextId++, who, text }]);
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
        case "delta":
          setMsgs((all) => {
            const last = all[all.length - 1];
            if (!last || last.who !== "assistant" || last.reqId !== m.id) return all;
            return [...all.slice(0, -1), { ...last, text: last.text + m.text }];
          });
          return;
        case "chat_done":
          setMsgs((all) => all.map((x) => (x.reqId === m.id ? { ...x, done: true } : x)));
          break;
        case "lines":
          push("system", `${m.title}\n${m.lines.join("\n")}`);
          break;
        case "map_built":
          push("system", `제품 지도를 다시 만들었습니다.\n${m.lines.join("\n")}`);
          void reload();
          break;
        case "progress":
          // One live line per request, updated in place.
          setMsgs((all) => {
            const i = all.findIndex((x) => x.reqId === m.id && x.who === "system");
            if (i < 0) return [...all, { id: nextId++, who: "system", text: m.text, reqId: m.id }];
            const next = all.slice();
            next[i] = { ...next[i], text: m.text };
            return next;
          });
          return;
        case "manual_done":
          push("system", `매뉴얼을 만들었습니다.\n${m.lines.join("\n")}`);
          setManualVersion((v) => v + 1);
          break;
        case "shots_done":
          push("system", `촬영을 마쳤습니다.\n${m.lines.join("\n")}`);
          void fetchShots().then(setShots);
          break;
        case "error":
          setMsgs((all) => all.filter((x) => !(x.reqId === m.id && x.who === "assistant" && !x.text)));
          push("error", m.message);
          break;
      }
      if (pending.current === m.id) {
        pending.current = null;
        setBusy(null);
      }
    },
    [push, reload],
  );

  useEffect(() => {
    void reload();
    const ch = new Channel(onMessage, setConnected);
    channel.current = ch;
    return () => ch.close();
  }, [onMessage, reload]);

  useEffect(() => {
    if (!map) return;
    setMsgs((m) =>
      m.length
        ? m
        : [
            {
              id: nextId++,
              who: "system",
              text: `제품 지도를 불러왔습니다: 기능 ${map.features.length}개, 화면 ${map.coverage.screens}개. 왼쪽에서 기능을 고르거나 질문하세요. / 를 입력하면 명령이 나옵니다.`,
            },
          ],
    );
  }, [map]);

  const feature = useMemo(() => map?.features.find((f) => f.key === selected) ?? null, [map, selected]);

  const runCommand = (name: string) => {
    const def = COMMANDS.find((c) => c.name === name);
    if (!def) return push("error", `알 수 없는 명령 /${name}. /help 를 보세요.`);
    if (name === "help") return push("system", HELP_LINES.join("\n"));
    if (name === "clear") return setMsgs([]);
    if (busy) return push("error", `${busy}이 끝난 뒤에 실행해 주세요.`);
    const id = nextId++;
    const arg = name === "gen" ? selected ?? undefined : undefined;
    if (name === "gen" && !arg) return push("error", "매뉴얼을 만들 기능을 왼쪽에서 먼저 고르세요.");
    if (!channel.current?.send({ type: "command", id, name, arg })) return push("error", "서버에 연결되어 있지 않습니다.");
    pending.current = id;
    setBusy({ map: "제품 지도 생성", doctor: "점검", shot: "촬영", gen: "매뉴얼 생성" }[name] ?? "조회");
  };

  const sendChat = (text: string) => {
    if (busy) return push("error", `${busy}이 끝난 뒤에 보내 주세요.`);
    const id = nextId++;
    const transcript = [
      ...msgs.filter((m) => (m.who === "user" || m.who === "assistant") && m.text).map((m) => ({ who: m.who, text: m.text })),
      { who: "user" as const, text },
    ];
    if (!channel.current?.send({ type: "chat", id, transcript, selected })) {
      return push("error", "서버에 연결되어 있지 않습니다.");
    }
    setMsgs((m) => [...m, { id: nextId++, who: "user", text }, { id: nextId++, who: "assistant", text: "", reqId: id }]);
    pending.current = id;
    setBusy("답변");
  };

  const changeModel = async (model: string) => {
    try {
      const status = await setChatModel(model);
      setState((s) => (s ? { ...s, status } : s));
      push("system", `대화 모델을 ${model}(으)로 바꿨습니다. 이 서버를 다시 켜면 mwa.toml 값으로 돌아갑니다.`);
    } catch (e) {
      push("error", String(e));
    }
  };

  const submit = (input: string) => {
    const t = input.trim();
    if (!t) return;
    if (t.startsWith("/")) runCommand(t.slice(1).split(/\s+/)[0]);
    else sendChat(t);
  };

  return (
    <div className="app">
      <header className="topbar">
        <span className="brand">mwa</span>
        <span className="muted mono">{state?.header.workspace ?? "…"}</span>
        <span className="spacer" />
        {state?.header.sources.map((s) => (
          <span key={s.name} className="source">
            <b>{s.name}</b> <span className="mono">{s.sha.slice(0, 7)}</span>
            {s.preview && <span className="badge warn">미리보기</span>}
          </span>
        ))}
      </header>
      <Sidebar map={map} selected={selected} onSelect={setSelected} />
      <main className={`content ${chatOpen ? "" : "chat-collapsed"}`}>
        <section className="detail">
          {loadError ? (
            <div className="empty error">서버에서 정보를 불러오지 못했습니다: {loadError}</div>
          ) : feature ? (
            <FeatureView
              feature={feature}
              permissions={map?.permissions ?? []}
              shot={shots?.shots.find((s) => s.key === feature.key) ?? null}
              shotsTakenAt={shots?.taken_at ?? null}
              manualVersion={manualVersion}
              onCommand={runCommand}
              onClose={() => setSelected(null)}
            />
          ) : (
            <Overview map={map} shots={shots} onCommand={runCommand} />
          )}
        </section>
        <button className="chat-toggle" onClick={() => setChatOpen((o) => !o)} title="대화창 접기/펼치기">
          {chatOpen ? "▾ 대화 접기" : "▴ 대화 펼치기"}
        </button>
        <Chat msgs={msgs} busy={busy} onSubmit={submit} featureLabel={feature?.label ?? null} onEscape={() => setSelected(null)} />
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
