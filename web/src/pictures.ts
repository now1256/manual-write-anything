import type { SceneShot } from "./api";

/** Which picture the new manual uses for an old-manual picture (D-032, D-033). */
export interface Picked {
  source: "upload" | "current" | "old";
  /** user | ok | diff | chosen | old */
  kind: string;
  label: string;
}

/**
 * A person's choice wins; otherwise automatic: upload, else a usable capture, else the old
 * picture (marked so it gets looked at).
 */
export function pickPicture(s: SceneShot | undefined, noCapture: boolean): Picked {
  const usable = !!s?.file && s.status !== "failed";
  const current = (): Picked =>
    s?.status === "reproduced"
      ? { source: "current", kind: "ok", label: "지금 화면" }
      : { source: "current", kind: "diff", label: "지금 화면 · 예전과 다름" };
  switch (s?.choice) {
    case "upload":
      if (s.uploaded) return { source: "upload", kind: "user", label: "올린 그림" };
      break;
    case "old":
      return { source: "old", kind: "chosen", label: "기존 그림 사용 (고름)" };
    case "current":
      if (s.file) return current();
      break;
  }
  if (s?.uploaded) return { source: "upload", kind: "user", label: "올린 그림" };
  if (usable) return current();
  const why = s && s.status !== "not_taken" ? "다시 찍지 못함" : noCapture ? "자동 촬영 대상 아님" : "아직 안 찍음";
  return { source: "old", kind: "old", label: `예전 화면 · ${why}` };
}
