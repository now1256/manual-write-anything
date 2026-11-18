import { useEffect, useRef, useState } from "react";
import { createProject, fetchProjects, openProject, type ProjectList } from "../api";

interface Props {
  /** Folder of the open project, from /api/state. */
  current: string | null;
}

type Mode = "list" | "new" | "open";

/** Top-bar project picker: list, create, open an existing folder. Switching reloads the page. */
export default function ProjectSwitcher({ current }: Props) {
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<Mode>("list");
  const [list, setList] = useState<ProjectList | null>(null);
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const box = useRef<HTMLDivElement>(null);

  const refresh = () =>
    fetchProjects()
      .then(setList)
      .catch((e) => setError(String(e.message ?? e)));

  useEffect(() => {
    refresh();
  }, [current]);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (box.current && !box.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  const currentName =
    list?.items.find((p) => p.path === current)?.name ?? current?.split("/").pop() ?? "프로젝트";

  const run = async (job: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await job();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setBusy(false);
    }
  };

  const switchTo = (p: string) =>
    run(async () => {
      if (p !== current) await openProject(p);
      window.location.reload();
    });

  const create = () =>
    run(async () => {
      const entry = await createProject(name.trim(), path.trim() || undefined);
      await openProject(entry.path);
      window.location.reload();
    });

  return (
    <div className="projects" ref={box}>
      <button className="project-button" onClick={() => (setOpen(!open), setMode("list"), setError(null))} title={current ?? ""}>
        <span className="project-name">{currentName}</span>
        <span className="muted">▾</span>
      </button>
      {open && (
        <div className="project-menu" role="dialog" aria-label="프로젝트">
          {mode === "list" && (
            <>
              <div className="project-items">
                {(list?.items ?? []).map((p) => (
                  <button
                    key={p.path}
                    className={`project-item ${p.path === current ? "active" : ""}`}
                    disabled={busy || !p.ready}
                    onClick={() => switchTo(p.path)}
                    title={p.ready ? p.path : `${p.path} (mwa.toml 없음)`}
                  >
                    <span>{p.name}</span>
                    <span className="muted mono small">{p.path}</span>
                  </button>
                ))}
                {list && list.items.length === 0 && <p className="muted pad">등록된 프로젝트가 없습니다.</p>}
              </div>
              {list?.switchable !== false && (
                <div className="project-actions">
                  <button onClick={() => (setMode("new"), setName(""), setPath(""))}>새 프로젝트</button>
                  <button onClick={() => (setMode("open"), setPath(""))}>기존 폴더 열기</button>
                </div>
              )}
            </>
          )}
          {mode === "new" && (
            <form
              className="project-form"
              onSubmit={(e) => {
                e.preventDefault();
                create();
              }}
            >
              <label>
                이름
                <input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="예: Acme 콘솔" />
              </label>
              <label>
                폴더 <span className="muted small">(비우면 기본 위치)</span>
                <input value={path} onChange={(e) => setPath(e.target.value)} placeholder="/절대/경로 또는 ~/경로" />
              </label>
              <div className="project-actions">
                <button type="button" onClick={() => setMode("list")}>취소</button>
                <button type="submit" className="primary" disabled={busy || !name.trim()}>
                  만들고 열기
                </button>
              </div>
            </form>
          )}
          {mode === "open" && (
            <form
              className="project-form"
              onSubmit={(e) => {
                e.preventDefault();
                switchTo(path.trim());
              }}
            >
              <label>
                mwa.toml이 있는 폴더
                <input autoFocus value={path} onChange={(e) => setPath(e.target.value)} placeholder="/절대/경로 또는 ~/경로" />
              </label>
              <div className="project-actions">
                <button type="button" onClick={() => setMode("list")}>취소</button>
                <button type="submit" className="primary" disabled={busy || !path.trim()}>
                  열기
                </button>
              </div>
            </form>
          )}
          {error && <p className="project-error">{error}</p>}
        </div>
      )}
    </div>
  );
}
