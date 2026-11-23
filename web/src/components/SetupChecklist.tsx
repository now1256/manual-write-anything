import { useEffect, useState } from "react";
import { fetchSetup, type SetupStep } from "../api";

interface Props {
  /** Bumped after commands finish so the count refreshes. */
  version: number;
  onOpen: () => void;
}

/** One line on the home screen while the project is not fully set up: opens the wizard. */
export default function SetupChecklist({ version, onOpen }: Props) {
  const [steps, setSteps] = useState<SetupStep[] | null>(null);
  useEffect(() => {
    fetchSetup().then(setSteps).catch(() => setSteps([]));
  }, [version]);
  if (!steps) return null;
  const mine = steps.filter((s) => s.key !== "manual");
  const done = mine.filter((s) => s.done).length;
  if (done === mine.length) return null;
  const next = mine.find((s) => !s.done);
  return (
    <section className="setup-line">
      <b>프로젝트 설정</b>
      <span className="muted small">
        {done}/{mine.length} 완료{next ? ` · 다음: ${next.label}` : ""}
      </span>
      <span className="spacer" />
      <button className="btn primary" onClick={onOpen}>설정 이어 하기 →</button>
    </section>
  );
}
