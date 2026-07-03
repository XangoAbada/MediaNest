import { useMemo } from "react";
import { useLibrary } from "../stores/library";
import { Grid } from "../components/Grid";

/** Przeglądarka folderów: breadcrumb + karty podfolderów + siatka plików. */
export function FoldersView() {
  const { q, setFolder, folders } = useLibrary();
  const parent = q.parent ?? "";

  // bezpośrednie podfoldery bieżącego folderu, z rekurencyjną sumą plików
  const children = useMemo(() => {
    const acc = new Map<string, number>();
    const prefix = parent ? `${parent}/` : "";
    for (const [path, count] of folders) {
      if (path === parent || !path) continue;
      if (parent && !path.startsWith(prefix)) continue;
      const name = (parent ? path.slice(prefix.length) : path).split("/")[0];
      if (!name) continue;
      acc.set(name, (acc.get(name) ?? 0) + count);
    }
    return [...acc.entries()].sort((a, b) => a[0].localeCompare(b[0], "pl"));
  }, [folders, parent]);

  const crumbs = parent ? parent.split("/") : [];

  return (
    <div className="flex h-full flex-col">
      {/* breadcrumb */}
      <div className="flex items-center gap-1 border-b border-edge bg-surface px-4 py-2 text-[13px]">
        <button
          onClick={() => setFolder("")}
          className={crumbs.length ? "text-ink-dim hover:text-ink" : "font-medium"}
        >
          Cała biblioteka
        </button>
        {crumbs.map((seg, i) => (
          <span key={i} className="flex items-center gap-1">
            <span className="text-ink-faint">/</span>
            <button
              onClick={() => setFolder(crumbs.slice(0, i + 1).join("/"))}
              className={
                i === crumbs.length - 1 ? "font-medium" : "text-ink-dim hover:text-ink"
              }
            >
              {seg}
            </button>
          </span>
        ))}
      </div>

      {/* karty podfolderów */}
      {children.length > 0 && (
        <div className="flex flex-wrap gap-2 border-b border-edge px-3 py-2">
          {children.map(([name, count]) => (
            <button
              key={name}
              onClick={() => setFolder(parent ? `${parent}/${name}` : name)}
              className="flex items-center gap-2 rounded-md border border-edge bg-surface px-3 py-1.5 text-[13px] text-ink-dim transition-colors hover:border-accent hover:text-ink"
            >
              <span className="opacity-70">📁</span>
              {name}
              <span className="text-[11px] text-ink-faint">{count.toLocaleString("pl")}</span>
            </button>
          ))}
        </div>
      )}

      <div className="min-h-0 flex-1">
        <Grid />
      </div>
    </div>
  );
}
