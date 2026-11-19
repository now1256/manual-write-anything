import { useEffect, useState } from "react";
import { fetchSetup, type SetupStep } from "../api";

interface Props {
  /** Bumped after commands finish so the list refreshes. */
  version: number;
  onCommand: (name: string) => void;
  /** Opens the settings with only this step's part. */
  onOpenSettings: (step: string) => void;
}

const COMMAND_LABEL: Record<string, string> = {
  sync: "Git 소스 받기",
  map: "지도 만들기",
  adapter: "코드 읽고 어댑터 만들기",
  shot: "첫 화면 촬영",
  doctor: "점검",
};

/** "시작하기": what a new project still needs, in order, with a button for each step. */
export default function SetupChecklist({ version, onCommand, onOpenSettings }: Props) {
  const [steps, setSteps] = useState<SetupStep[] | null>(null);
  const [all, setAll] = useState(false);

  useEffect(() => {
    fetchSetup().then(setSteps).catch(() => setSteps([]));
  }, [version]);

  if (!steps || steps.length === 0 || steps.every((s) => s.done)) return null;
  const next = steps.find((s) => !s.done)?.key;

  return (
    <section className="setup">
      <div className="setup-head">
        <h2>시작하기</h2>
        <span className="muted small">
          {steps.filter((s) => s.done).length}/{steps.length}단계 완료
        </span>
        <span className="spacer" />
        <button className="link" onClick={() => setAll(!all)}>{all ? "남은 단계만" : "모든 단계 보기"}</button>
      </div>
      <ol>
        {steps.filter((s) => all || !s.done).map((s) => {
          const command = s.action.startsWith("command:") ? s.action.slice(8) : null;
          return (
            <li key={s.key} className={`${s.done ? "done" : ""} ${s.key === next ? "next" : ""}`}>
              <span className="mark">{s.done ? "✓" : "○"}</span>
              <span className="what">
                <b>{s.label}</b>
                <span className="muted">{s.detail}</span>
              </span>
              {s.key === next && s.action === "settings" && (
                <button className="primary" onClick={() => onOpenSettings(s.key)}>설정하기</button>
              )}
              {s.key === next && command && (
                <button className="primary" onClick={() => onCommand(command)}>
                  {COMMAND_LABEL[command] ?? `/${command}`}
                </button>
              )}
            </li>
          );
        })}
      </ol>
      <div className="setup-foot">
        <button onClick={() => onCommand("doctor")}>전체 점검</button>
      </div>
    </section>
  );
}
