import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useApp, type View } from "../stores/app";
import { useLibrary, type Sort } from "../stores/library";
import { EmptyState } from "./EmptyState";
import { FolderPicker } from "./FolderPicker";
import { FolderTree } from "./FolderTree";
import { Grid } from "./Grid";
import { Lightbox } from "./Lightbox";
import { PasswordPrompt } from "./PasswordPrompt";
import { SettingsView } from "../views/SettingsView";
import { ImportView } from "../views/ImportView";
import { TrashView } from "../views/TrashView";
import { DuplicatesView } from "../views/DuplicatesView";
import { AlbumsView } from "../views/AlbumsView";
import { TagsView } from "../views/TagsView";
import { TimelineView } from "../views/TimelineView";
import { FoldersView } from "../views/FoldersView";

interface NavItem {
  view: View;
  label: string;
  icon: string;
}

const SECTIONS: { title: string; items: NavItem[] }[] = [
  {
    title: "Biblioteka",
    items: [
      { view: "all", label: "Wszystkie pliki", icon: "🖼" },
      { view: "folders", label: "Foldery", icon: "📁" },
      { view: "timeline", label: "Oś czasu", icon: "📅" },
    ],
  },
  {
    title: "Kolekcje",
    items: [
      { view: "albums", label: "Albumy", icon: "📚" },
      { view: "tags", label: "Tagi", icon: "🏷" },
    ],
  },
  {
    title: "Narzędzia",
    items: [
      { view: "import", label: "Import", icon: "⬇" },
      { view: "duplicates", label: "Duplikaty", icon: "⧉" },
      { view: "trash", label: "Kosz", icon: "🗑" },
    ],
  },
];

const VIEW_TITLES: Record<View, string> = {
  all: "Wszystkie pliki",
  timeline: "Oś czasu",
  folders: "Foldery",
  albums: "Albumy",
  tags: "Tagi",
  duplicates: "Duplikaty",
  import: "Import",
  trash: "Kosz",
  settings: "Ustawienia",
};

function Toasts() {
  const { toasts, dismissToast } = useApp();
  if (toasts.length === 0) return null;
  return (
    <div className="fixed bottom-4 right-4 z-[60] flex w-80 flex-col gap-2">
      {toasts.map((t) => (
        <div
          key={t.id}
          className="flex items-center gap-3 rounded-lg border border-edge bg-raised px-4 py-3 text-[13px] shadow-lg"
        >
          <span className="min-w-0 flex-1">{t.text}</span>
          {t.action && (
            <button
              onClick={() => {
                t.action!.run();
                dismissToast(t.id);
              }}
              className="shrink-0 rounded-md bg-accent px-2.5 py-1 text-[12px] text-white hover:bg-accent-hover"
            >
              {t.action.label}
            </button>
          )}
          <button
            onClick={() => dismissToast(t.id)}
            className="shrink-0 text-ink-faint hover:text-ink"
          >
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}

function NavButton({
  item,
  active,
  onClick,
}: {
  item: NavItem;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors duration-100 ${
        active ? "bg-accent/15 text-ink" : "text-ink-dim hover:bg-raised hover:text-ink"
      }`}
    >
      <span className="w-4 text-center text-xs opacity-80">{item.icon}</span>
      {item.label}
    </button>
  );
}

/** Pasek akcji dla zaznaczenia wielu plików. */
function SelectionBar() {
  const { selection, clearSelection, refresh } = useLibrary();
  const toast = useApp((s) => s.toast);
  const [albums, setAlbums] = useState<{ id: number; name: string }[]>([]);
  const [movingToFolder, setMovingToFolder] = useState(false);

  useEffect(() => {
    if (selection.size > 0) {
      invoke<{ id: number; name: string }[]>("list_albums").then(setAlbums);
    }
  }, [selection.size > 0]);

  if (selection.size === 0) return null;
  const ids = [...selection];

  return (
    <div className="flex items-center gap-3 border-b border-edge bg-raised px-4 py-2 text-[13px]">
      <span className="font-medium">{selection.size} zaznaczonych</span>
      <select
        defaultValue=""
        onChange={async (e) => {
          const albumId = Number(e.target.value);
          if (!albumId) return;
          const n = await invoke<number>("add_to_album", { albumId, fileIds: ids });
          toast(`Dodano ${n} plików do albumu`);
          e.target.value = "";
          clearSelection();
          refresh();
        }}
        className="rounded-md border border-edge bg-surface px-2 py-1 text-[12px] outline-none"
      >
        <option value="">Dodaj do albumu…</option>
        {albums.map((a) => (
          <option key={a.id} value={a.id}>
            {a.name}
          </option>
        ))}
      </select>
      <select
        defaultValue=""
        onChange={async (e) => {
          if (e.target.value === "") return;
          const rating = Number(e.target.value);
          await Promise.all(ids.map((id) => invoke("set_rating", { id, rating })));
          toast(`Oceniono ${ids.length} plików`);
          e.target.value = "";
          clearSelection();
          refresh();
        }}
        className="rounded-md border border-edge bg-surface px-2 py-1 text-[12px] outline-none"
      >
        <option value="">Oceń…</option>
        {[5, 4, 3, 2, 1, 0].map((r) => (
          <option key={r} value={r}>
            {r === 0 ? "Wyczyść ocenę" : "★".repeat(r)}
          </option>
        ))}
      </select>
      <button
        onClick={() => setMovingToFolder(true)}
        className="rounded-md border border-edge px-3 py-1 hover:border-ink-faint"
      >
        Przenieś do folderu…
      </button>
      <button
        onClick={async () => {
          const n = await invoke<number>("trash_files", { ids });
          toast(`Przeniesiono ${n} plików do kosza`);
          clearSelection();
          refresh();
        }}
        className="rounded-md border border-danger/40 px-3 py-1 text-danger hover:bg-danger/10"
      >
        Do kosza
      </button>
      <button
        onClick={clearSelection}
        className="ml-auto text-ink-faint hover:text-ink"
      >
        Wyczyść zaznaczenie
      </button>
      {movingToFolder && (
        <FolderPicker ids={ids} onClose={() => setMovingToFolder(false)} />
      )}
    </div>
  );
}

function Toolbar() {
  const { view } = useApp();
  const { q, setQuery, cellSize, setCellSize, total, indexing } = useLibrary();
  const [search, setSearch] = useState("");
  const gridView = view === "all" || view === "folders";

  // debounce wyszukiwania
  useEffect(() => {
    const t = setTimeout(() => {
      const query = search.trim() || null;
      if (query !== q.query) setQuery({ query });
    }, 300);
    return () => clearTimeout(t);
  }, [search]);

  return (
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-edge bg-surface px-4">
      <span className="shrink-0 text-[13px] font-medium">
        {view === "folders" ? q.parent?.split("/").pop() || "Cała biblioteka" : VIEW_TITLES[view]}
      </span>
      {gridView && (
        <>
          <span className="shrink-0 text-[12px] text-ink-faint">
            {total.toLocaleString("pl")} plików
          </span>
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Szukaj (nazwa, folder, tag)…"
            className="w-56 rounded-md border border-edge bg-app px-2.5 py-1 text-[12px] outline-none placeholder:text-ink-faint focus:border-accent"
          />
          <select
            value={q.kind === null ? "" : String(q.kind)}
            onChange={(e) =>
              setQuery({ kind: e.target.value === "" ? null : Number(e.target.value) })
            }
            className="rounded-md border border-edge bg-raised px-2 py-1 text-[12px] text-ink-dim outline-none"
          >
            <option value="">Wszystko</option>
            <option value="0">Zdjęcia</option>
            <option value="1">Wideo</option>
          </select>
          <select
            value={q.rating_min}
            onChange={(e) => setQuery({ rating_min: Number(e.target.value) })}
            className="rounded-md border border-edge bg-raised px-2 py-1 text-[12px] text-ink-dim outline-none"
          >
            <option value="0">Dowolna ocena</option>
            {[1, 2, 3, 4, 5].map((r) => (
              <option key={r} value={r}>
                {"★".repeat(r)}+
              </option>
            ))}
          </select>
          <select
            value={q.sort}
            onChange={(e) => setQuery({ sort: e.target.value as Sort })}
            className="rounded-md border border-edge bg-raised px-2 py-1 text-[12px] text-ink-dim outline-none"
          >
            <option value="date_desc">Najnowsze</option>
            <option value="date_asc">Najstarsze</option>
            <option value="name">Nazwa</option>
            <option value="size_desc">Rozmiar</option>
          </select>
          {view === "folders" && (
            <label className="flex cursor-pointer items-center gap-1.5 text-[12px] text-ink-dim">
              <input
                type="checkbox"
                checked={q.recursive}
                onChange={(e) => setQuery({ recursive: e.target.checked })}
                className="accent-(--color-accent)"
              />
              Z podfolderami
            </label>
          )}
          <div className="ml-auto flex items-center gap-2">
            {indexing && (
              <div className="flex items-center gap-2 text-[12px] text-ink-dim">
                <span>Indeksowanie… {indexing.pending.toLocaleString("pl")}</span>
                <div className="h-1 w-24 overflow-hidden rounded-full bg-raised">
                  <div
                    className="h-full bg-accent transition-[width] duration-500"
                    style={{
                      width: `${
                        indexing.total > 0
                          ? Math.round(
                              (100 * (indexing.total - indexing.pending)) / indexing.total,
                            )
                          : 0
                      }%`,
                    }}
                  />
                </div>
              </div>
            )}
            <input
              type="range"
              min={96}
              max={256}
              step={16}
              value={cellSize}
              onChange={(e) => setCellSize(Number(e.target.value))}
              className="w-28 accent-(--color-accent)"
              title="Rozmiar miniaturek"
            />
          </div>
        </>
      )}
    </header>
  );
}

export function AppShell() {
  const { view, setView, toast, setImportSource } = useApp();
  const { resetQuery, refresh, setIndexing, lightbox } = useLibrary();
  const refreshTimer = useRef(0);

  useEffect(() => {
    refresh();
    const unProgress = listen<{ pending: number; total: number }>(
      "index-progress",
      (event) => {
        setIndexing(event.payload);
        const now = Date.now();
        if (event.payload.pending === 0 || now - refreshTimer.current > 3000) {
          refreshTimer.current = now;
          refresh();
        }
      },
    );
    const unChanged = listen("library-changed", () => refresh());
    const unDrive = listen<string>("drive-added", (event) => {
      toast(`Wykryto nośnik ${event.payload}`, {
        label: "Importuj",
        run: () => {
          setImportSource(event.payload);
          useApp.getState().setView("import");
        },
      });
    });
    return () => {
      unProgress.then((fn) => fn());
      unChanged.then((fn) => fn());
      unDrive.then((fn) => fn());
    };
  }, [refresh, setIndexing, toast, setImportSource]);

  // auto-blokada chronionych albumów po 15 min bezczynności
  useEffect(() => {
    let timer = 0;
    const reset = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        invoke("lock_albums");
      }, 15 * 60 * 1000);
    };
    reset();
    window.addEventListener("pointermove", reset);
    window.addEventListener("keydown", reset);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("pointermove", reset);
      window.removeEventListener("keydown", reset);
    };
  }, []);

  return (
    <div className="flex h-full">
      <aside className="flex w-60 shrink-0 flex-col border-r border-edge bg-surface">
        <div className="flex items-center gap-2 px-4 py-4">
          <span className="text-xl">🪺</span>
          <span className="text-[15px] font-semibold tracking-tight">MediaNest</span>
        </div>
        <nav className="flex-1 overflow-y-auto px-2">
          {SECTIONS.map((section) => (
            <div key={section.title} className="mb-4">
              <div className="px-2 pb-1 text-[11px] font-semibold uppercase tracking-wider text-ink-faint">
                {section.title}
              </div>
              {section.items.map((item) => (
                <NavButton
                  key={item.view}
                  item={item}
                  active={view === item.view}
                  onClick={() => {
                    if (item.view === "all") resetQuery();
                    setView(item.view);
                  }}
                />
              ))}
              {section.title === "Biblioteka" && (
                <div className="mt-1">
                  <div className="px-2 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wider text-ink-faint">
                    Foldery
                  </div>
                  <FolderTree />
                </div>
              )}
            </div>
          ))}
        </nav>
        <div className="border-t border-edge p-2">
          <NavButton
            item={{ view: "settings", label: "Ustawienia", icon: "⚙" }}
            active={view === "settings"}
            onClick={() => setView("settings")}
          />
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <Toolbar />
        <SelectionBar />
        <main className="min-h-0 flex-1">
          {view === "settings" ? (
            <SettingsView />
          ) : view === "import" ? (
            <ImportView />
          ) : view === "trash" ? (
            <TrashView />
          ) : view === "duplicates" ? (
            <DuplicatesView />
          ) : view === "albums" ? (
            <AlbumsView />
          ) : view === "tags" ? (
            <TagsView />
          ) : view === "timeline" ? (
            <TimelineView />
          ) : view === "folders" ? (
            <FoldersView />
          ) : view === "all" ? (
            <Grid />
          ) : (
            <EmptyState icon="•" title={VIEW_TITLES[view]} />
          )}
        </main>
      </div>
      {lightbox !== null && <Lightbox />}
      <PasswordPrompt />
      <Toasts />
    </div>
  );
}
