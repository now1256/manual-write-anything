import { useMemo, useState } from "react";
import type { Feature, ProductMap } from "../api";

interface Props {
  map: ProductMap | null;
  selected: string | null;
  onSelect: (key: string) => void;
}

export default function Sidebar({ map, selected, onSelect }: Props) {
  const [query, setQuery] = useState("");
  const groups = useMemo(() => {
    const out: { name: string; items: Feature[] }[] = [];
    for (const f of map?.features ?? []) {
      if (query && !`${f.label} ${f.href} ${f.group ?? ""}`.toLowerCase().includes(query.toLowerCase())) continue;
      // Sections without groups (e.g. settings-only menus) are named by their license menu.
      const name = f.group ?? f.license_menu;
      const last = out[out.length - 1];
      if (last?.name === name) last.items.push(f);
      else out.push({ name, items: [f] });
    }
    return out;
  }, [map, query]);

  return (
    <nav className="sidebar">
      <div className="sidebar-head">
        <span>기능 {map?.features.length ?? 0}</span>
      </div>
      <input className="search" placeholder="기능 찾기" value={query} onChange={(e) => setQuery(e.target.value)} />
      {!map && <p className="muted pad">기능 목록은 시작하기의 "코드 읽고 제품 파악" 뒤에 나타납니다.</p>}
      <div className="tree">
        {groups.map((g) => (
          <div key={g.name} className="group">
            <div className="group-name">{g.name}</div>
            {g.items.map((f) => {
              const apis = f.screens.reduce((n, s) => n + s.apis.length, 0);
              return (
                <button
                  key={f.key}
                  className={`item ${f.key === selected ? "active" : ""}`}
                  onClick={() => onSelect(f.key)}
                  title={f.href}
                >
                  <span className="label">{f.label}</span>
                  <span className="count">{apis}</span>
                </button>
              );
            })}
          </div>
        ))}
      </div>
    </nav>
  );
}
