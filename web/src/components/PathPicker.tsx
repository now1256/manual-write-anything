import { useEffect, useState } from "react";
import { browseFolder, type FolderListing } from "../api";

interface Props {
  /** "dir" picks a folder; "file" picks a file with one of `ext` (e.g. "toml"). */
  mode: "dir" | "file";
  ext?: string;
  start?: string;
  onPick: (path: string) => void;
  onClose: () => void;
}

/** Browses folders on the engine's machine (read only). */
export default function PathPicker({ mode, ext = "", start, onPick, onClose }: Props) {
  const [listing, setListing] = useState<FolderListing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [typed, setTyped] = useState("");

  const go = (path: string | null) =>
    browseFolder(path, mode === "file" ? ext : "")
      .then((l) => {
        setListing(l);
        setTyped(l.path);
        setError(null);
      })
      .catch((e) => setError(String(e.message ?? e)));

  useEffect(() => {
    const dir = start && mode === "file" ? start.replace(/\/[^/]*$/, "") : start;
    go(dir || null).catch(() => go(null));
  }, []);

  const join = (name: string) => `${listing!.path.replace(/\/$/, "")}/${name}`;

  return (
    <div className="picker-backdrop" onMouseDown={onClose}>
      <div className="picker" role="dialog" aria-label="경로 선택" onMouseDown={(e) => e.stopPropagation()}>
        <div className="picker-head">
          <b>{mode === "dir" ? "폴더 선택" : ext === "*" ? "파일 선택" : `파일 선택 (.${ext})`}</b>
          <span className="muted small">엔진이 도는 컴퓨터의 경로입니다</span>
        </div>
        <form
          className="picker-path"
          onSubmit={(e) => {
            e.preventDefault();
            go(typed);
          }}
        >
          <button type="button" disabled={!listing?.parent} onClick={() => go(listing!.parent)} title="상위 폴더">
            ↑
          </button>
          <input value={typed} onChange={(e) => setTyped(e.target.value)} />
          <button type="submit">이동</button>
        </form>
        {error && <p className="settings-error">{error}</p>}
        <div className="picker-list">
          {listing?.entries.map((e) => (
            <button
              key={e.name}
              className="picker-item"
              onClick={() => (e.dir ? go(join(e.name)) : onPick(join(e.name)))}
            >
              <span>{e.dir ? "📁" : "📄"}</span>
              <span className="grow">{e.name}</span>
              {e.git && <span className="badge ok">git</span>}
            </button>
          ))}
          {listing && listing.entries.length === 0 && <p className="muted pad">비어 있습니다</p>}
        </div>
        <div className="picker-actions">
          <button onClick={onClose}>취소</button>
          {mode === "dir" && (
            <button className="primary" disabled={!listing} onClick={() => onPick(listing!.path)}>
              이 폴더 선택
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
