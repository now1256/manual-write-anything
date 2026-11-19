import { useEffect, useState } from "react";

export interface Run {
  id: number;
  label: string;
  /** Latest progress line from the engine. */
  line: string;
  started: number;
}

export interface Notice {
  id: number;
  ok: boolean;
  title: string;
  lines: string[];
}

/** Top of the page: the running job with its latest step, then results until closed. */
export default function RunBanner({ run, notices, onClose }: { run: Run | null; notices: Notice[]; onClose: (id: number) => void }) {
  const [, tick] = useState(0);
  useEffect(() => {
    if (!run) return;
    const t = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [run]);
  if (!run && notices.length === 0) return null;
  const secs = run ? Math.floor((Date.now() - run.started) / 1000) : 0;
  return (
    <div className="run-banner">
      {run && (
        <div className="run">
          <span className="spinner" />
          <b>{run.label}</b>
          <span className="run-line">{run.line || "시작하는 중…"}</span>
          <span className="spacer" />
          <span className="muted small mono">{Math.floor(secs / 60)}분 {secs % 60}초</span>
          <div className="run-bar" />
        </div>
      )}
      {notices.map((n) => (
        <div key={n.id} className={`notice ${n.ok ? "ok" : "err"}`}>
          <div className="notice-head">
            <b>{n.ok ? "✓" : "✗"} {n.title}</b>
            <span className="spacer" />
            <button className="notice-x" onClick={() => onClose(n.id)} title="닫기">×</button>
          </div>
          {n.lines.length > 0 && (
            <ul>
              {n.lines.map((l, i) => (
                <li key={i}>{l}</li>
              ))}
            </ul>
          )}
        </div>
      ))}
    </div>
  );
}
