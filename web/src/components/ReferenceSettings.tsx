import { useEffect, useState } from "react";
import { fetchReferences, importReferences, setReference, type RefList } from "../api";
import PathPicker from "./PathPicker";

/** Settings section: import old manuals (PDF) and tell which feature each documents. Acts at once. */
export default function ReferenceSettings() {
  const [list, setList] = useState<RefList | null>(null);
  const [picker, setPicker] = useState<"file" | "dir" | null>(null);
  const [busy, setBusy] = useState(false);
  const [lines, setLines] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = () =>
    fetchReferences()
      .then(setList)
      .catch((e) => setError(String(e.message ?? e)));

  useEffect(() => {
    refresh();
  }, []);

  const doImport = async (path: string) => {
    setBusy(true);
    setError(null);
    try {
      const r = await importReferences([path]);
      setLines(r.lines);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const choose = async (id: string, value: string) => {
    try {
      await setReference(id, value === "-" || value === "" ? null : value, value === "-");
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const features = list?.features ?? [];
  return (
    <section>
      <h3>기존 매뉴얼 (PDF)</h3>
      <p className="muted small">
        예전에 만든 매뉴얼 PDF를 가져와 기능마다 짝짓습니다. 캡처는 "찍어야 할 장면" 참고와 대조에만 쓰고, 내용의 근거는 코드입니다. 이 구역은 바로 반영됩니다.
      </p>
      <div className="settings-actions">
        <button disabled={busy} onClick={() => setPicker("file")}>PDF 파일 추가</button>
        <button disabled={busy} onClick={() => setPicker("dir")}>폴더의 PDF 모두 가져오기</button>
        {busy && <span className="muted small">가져오는 중…</span>}
      </div>
      {error && <p className="settings-error">{error}</p>}
      {lines.length > 0 && <pre className="ref-lines">{lines.join("\n")}</pre>}
      {list && list.items.length > 0 && (
        <table className="roles">
          <thead>
            <tr><th>기존 매뉴얼</th><th>쪽 · 캡처</th><th>기능</th></tr>
          </thead>
          <tbody>
            {list.items.map((d) => (
              <tr key={d.id}>
                <td>
                  {d.title}
                  {d.tag ? ` (${d.tag})` : ""}
                  {d.described > 0 && <span className="badge ok">장면 {d.described}</span>}
                </td>
                <td className="muted">{d.pages}쪽 · {d.images}장</td>
                <td>
                  <select
                    value={d.ignored ? "-" : d.feature ?? ""}
                    onChange={(e) => choose(d.id, e.target.value)}
                    className={!d.feature && !d.ignored ? "need" : ""}
                  >
                    <option value="">기능을 고르세요</option>
                    <option value="-">기능 아님 (예: 로그인)</option>
                    {features.map((f) => (
                      <option key={f.key} value={f.key}>
                        {f.group ? `${f.group} › ` : ""}{f.label}
                      </option>
                    ))}
                  </select>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {list && list.items.length > 0 && features.length === 0 && (
        <p className="muted small">제품 지도가 아직 없어 기능 목록이 비어 있습니다. 지도를 만든 뒤 짝지으세요.</p>
      )}
      {picker && (
        <PathPicker
          mode={picker}
          ext="pdf"
          onClose={() => setPicker(null)}
          onPick={(p) => {
            setPicker(null);
            doImport(p);
          }}
        />
      )}
    </section>
  );
}
