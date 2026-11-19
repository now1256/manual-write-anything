import { docAssetUrl, refFileUrl, shotUrl, type SceneShot, type SceneShots } from "./api";

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

/** What a figure shows: its picture, how it was chosen, and the old manual's picture. */
export interface Pic {
  src: string | null;
  kind: "user" | "ok" | "diff" | "chosen" | "old" | "none";
  label: string;
  /** The old manual's picture, for comparison. */
  old: string | null;
}

/**
 * The picture of a figure (`data-ref` old picture, `data-asset` upload, `data-shot` capture), the
 * same in the editor, the Word export and Confluence publishing.
 */
export const pictureResolver =
  (featureKey: string, reference: string | null, shots: SceneShots | null, noCapture: Set<string>) =>
  (ref: string | null, asset: string | null, shot: string | null): Pic => {
    const old = ref && reference ? refFileUrl(reference, ref) : null;
    if (asset) return { src: docAssetUrl(featureKey, asset), kind: "user", label: "올린 그림", old };
    if (ref) {
      const s = shots?.scenes.find((x) => x.image === ref);
      const p = pickPicture(s, noCapture.has(ref));
      const src =
        p.source === "upload" && s?.uploaded
          ? docAssetUrl(featureKey, s.uploaded)
          : p.source === "current" && s?.file
            ? shotUrl(s.file, String(shots!.taken_at))
            : old;
      return { src, kind: p.kind as Pic["kind"], label: p.label, old: p.source === "old" ? null : old };
    }
    if (shot) return { src: shotUrl(shot, null), kind: "ok", label: "현재 화면", old: null };
    return { src: null, kind: "none", label: "그림 없음", old: null };
  };
