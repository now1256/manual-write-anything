import { useEffect, useState } from "react";
import { fetchConfluence, fetchDoc, fetchReference, fetchReferences, fetchSceneShots, publishConfluence } from "../api";
import { toStorage } from "../confluenceStorage";
import { pictureResolver } from "../pictures";

/**
 * Publishing without a person (자동 작업 "게시까지"): the engine opens this page in a headless
 * browser, which converts and publishes exactly as the editor's button does. The outcome goes
 * into the page title for the engine to read.
 */
export default function AutoPublish({ feature, group }: { feature: string; group: string | null }) {
  const [text, setText] = useState("게시 중…");
  useEffect(() => {
    const done = (t: string) => {
      document.title = t;
      setText(t);
    };
    (async () => {
      const doc = await fetchDoc(feature);
      if (!doc) throw new Error("매뉴얼 문서가 없습니다");
      const st = await fetchConfluence(feature);
      if (!st.enabled || !st.configured || !st.has_token) throw new Error("컨플루언스 게시가 설정되지 않았습니다 (④에서 한 번 설정)");
      const shots = await fetchSceneShots(feature).catch(() => null);
      const refId = doc.reference ?? (await fetchReferences()).items.find((d) => d.feature === feature && !d.ignored)?.id ?? null;
      const r = refId ? await fetchReference(refId).catch(() => null) : null;
      const noCapture = new Set((r?.images ?? []).filter((i) => i.scene && !i.scene.capture).map((i) => i.file));
      const resolve = pictureResolver(feature, refId, shots, noCapture);
      const converted = toStorage(doc.html, (fig) => resolve(fig.dataset.ref ?? null, fig.dataset.asset ?? null, fig.dataset.shot ?? null).src, doc.title);
      const p = await publishConfluence(feature, {
        title: st.published?.title ?? doc.title,
        storage: converted.storage,
        images: converted.images,
        adopt: false,
        group,
      });
      done(`mwa-done ${p.url}`);
    })().catch((e) => done(`mwa-error ${String((e as Error).message ?? e)}`));
  }, [feature, group]);
  return <p style={{ padding: 16 }}>{text}</p>;
}
