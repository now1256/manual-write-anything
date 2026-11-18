import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { Who } from "../api";
import { COMMANDS } from "../commands";

export interface ChatMsg {
  id: number;
  who: Who;
  text: string;
  /** Request id for assistant replies being streamed. */
  reqId?: number;
  done?: boolean;
}

interface Props {
  msgs: ChatMsg[];
  busy: string | null;
  featureLabel: string | null;
  onSubmit: (text: string) => void;
  onEscape: () => void;
}

const PREFIX: Record<Who, string> = { user: "›", assistant: "●", system: "·", error: "!" };

export default function Chat({ msgs, busy, featureLabel, onSubmit, onEscape }: Props) {
  const [input, setInput] = useState("");
  const [pick, setPick] = useState(0);
  const log = useRef<HTMLDivElement>(null);
  const box = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    const el = log.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [msgs]);

  const palette = useMemo(() => {
    if (!input.startsWith("/") || input.includes(" ") || input.includes("\n")) return [];
    const q = input.slice(1);
    return COMMANDS.filter((c) => c.name.startsWith(q));
  }, [input]);

  useEffect(() => setPick(0), [palette.length]);

  const send = (text: string) => {
    onSubmit(text);
    setInput("");
  };

  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return; // Korean IME composition in progress
    if (palette.length) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const d = e.key === "ArrowDown" ? 1 : -1;
        setPick((p) => (p + d + palette.length) % palette.length);
        return;
      }
      if (e.key === "Tab") {
        e.preventDefault();
        setInput(`/${palette[pick].name}`);
        return;
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        send(`/${palette[pick].name}`);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send(input);
    } else if (e.key === "Escape") {
      if (input) setInput("");
      else onEscape();
    }
  };

  return (
    <section className="chat">
      <div className="chat-log mono" ref={log} aria-live="polite">
        {msgs.map((m) => (
          <div key={m.id} className={`line ${m.who}`}>
            <span className="prefix">{PREFIX[m.who]}</span>
            <span className="text">
              {m.text || (m.who === "assistant" && !m.done ? "" : m.text)}
              {m.who === "assistant" && !m.done && <span className="cursor">▍</span>}
            </span>
          </div>
        ))}
      </div>
      <div className="composer">
        {palette.length > 0 && (
          <ul className="palette" role="listbox">
            {palette.map((c, i) => (
              <li
                key={c.name}
                role="option"
                aria-selected={i === pick}
                className={i === pick ? "on" : ""}
                onMouseDown={(e) => {
                  e.preventDefault();
                  send(`/${c.name}`);
                }}
              >
                <span className="mono">/{c.name}</span>
                <span className="muted">{c.desc}</span>
              </li>
            ))}
          </ul>
        )}
        <span className="prompt mono">&gt;</span>
        <textarea
          ref={box}
          className="mono"
          rows={1}
          value={input}
          placeholder={busy ? `${busy} 중…` : featureLabel ? `${featureLabel}에 대해 묻거나 / 로 명령` : "질문하거나 / 로 명령"}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={onKey}
          autoFocus
        />
      </div>
    </section>
  );
}
