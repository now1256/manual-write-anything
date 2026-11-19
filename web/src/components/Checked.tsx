import { useEffect, useState } from "react";
import { checkField, type Check } from "../api";

interface Props {
  kind: string;
  value: string;
  extra?: string;
}

/** Live ✓/✗ under a settings field: probes the engine a moment after the value settles. */
export default function Checked({ kind, value, extra = "" }: Props) {
  const [result, setResult] = useState<Check | null>(null);
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    let alive = true;
    setChecking(true);
    const t = setTimeout(() => {
      checkField(kind, value, extra)
        .then((r) => alive && setResult(r))
        .catch((e) => alive && setResult({ ok: false, detail: String(e.message ?? e) }))
        .finally(() => alive && setChecking(false));
    }, 600);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [kind, value, extra]);

  if (checking && !result) return <div className="check muted">확인 중…</div>;
  if (!result) return null;
  return (
    <div className={`check ${result.ok ? "ok" : "bad"}`}>
      {result.ok ? "✓" : "✗"} {result.detail}
      {checking && <span className="muted"> · 다시 확인 중…</span>}
    </div>
  );
}
