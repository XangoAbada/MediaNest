import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useLibrary } from "../stores/library";
import { useApp } from "../stores/app";

/**
 * Wspólny wybierak folderu docelowego. Przenosi `ids` do wybranego folderu
 * (istniejącego, korzenia lub nowo utworzonego) i zamyka się.
 * ponytail: filtrowana płaska lista folderów; drzewo (FolderTree z onSelect)
 * dopiero gdyby filtr okazał się niewygodny.
 */
export function FolderPicker({
  ids,
  onClose,
  onMoved,
}: {
  ids: number[];
  onClose: () => void;
  onMoved?: () => void;
}) {
  const folders = useLibrary((s) => s.folders);
  const { clearSelection, refresh } = useLibrary();
  const toast = useApp((s) => s.toast);
  const [filter, setFilter] = useState("");
  const [newName, setNewName] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  const paths = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return folders
      .map(([p]) => p)
      .filter((p) => p && p.toLowerCase().includes(f))
      .sort((a, b) => a.localeCompare(b, "pl"));
  }, [folders, filter]);

  async function move(dst: string) {
    if (busy) return;
    setBusy(true);
    try {
      const n = await invoke<number>("move_to_folder", { dst, fileIds: ids });
      toast(`Przeniesiono ${n} plików`);
      clearSelection();
      refresh();
      onMoved?.();
      onClose();
    } catch (e) {
      toast(String(e));
      setBusy(false);
    }
  }

  async function createAndMove() {
    const name = (newName ?? "").trim();
    if (!name || busy) return;
    setBusy(true);
    try {
      const rel = await invoke<string>("create_folder", { parent: "", name });
      const n = await invoke<number>("move_to_folder", { dst: rel, fileIds: ids });
      toast(`Przeniesiono ${n} plików do „${name}"`);
      clearSelection();
      refresh();
      onMoved?.();
      onClose();
    } catch (e) {
      toast(String(e));
      setBusy(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/50"
      onClick={onClose}
    >
      <div
        className="flex max-h-[70vh] w-96 flex-col rounded-lg border border-edge bg-surface shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="border-b border-edge px-4 py-3 text-[13px] font-medium">
          Przenieś {ids.length}{" "}
          {ids.length === 1 ? "plik" : "plików"} do folderu
        </div>
        <div className="p-3">
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            autoFocus
            placeholder="Szukaj folderu…"
            className="w-full rounded-md border border-edge bg-app px-2.5 py-1.5 text-[13px] outline-none focus:border-accent"
          />
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          <button
            onClick={() => move("")}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] text-ink-dim hover:bg-raised hover:text-ink"
          >
            <span className="opacity-70">🏠</span> Cała biblioteka (korzeń)
          </button>
          {paths.map((p) => (
            <button
              key={p}
              onClick={() => move(p)}
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] text-ink-dim hover:bg-raised hover:text-ink"
            >
              <span className="opacity-70">📁</span>
              <span className="truncate">{p}</span>
            </button>
          ))}
          {paths.length === 0 && filter.trim() && (
            <div className="px-2 py-2 text-[12px] text-ink-faint">Brak pasujących folderów</div>
          )}
        </div>
        <div className="border-t border-edge p-3">
          {newName === null ? (
            <button
              onClick={() => setNewName("")}
              className="text-[13px] text-accent hover:text-accent-hover"
            >
              ＋ Nowy folder…
            </button>
          ) : (
            <div className="flex items-center gap-2">
              <input
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && createAndMove()}
                autoFocus
                placeholder="Nazwa nowego folderu"
                className="flex-1 rounded-md border border-edge bg-app px-2.5 py-1.5 text-[13px] outline-none focus:border-accent"
              />
              <button
                onClick={createAndMove}
                disabled={!newName.trim() || busy}
                className="rounded-md bg-accent px-3 py-1.5 text-[13px] text-white hover:bg-accent-hover disabled:opacity-50"
              >
                Utwórz i przenieś
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
