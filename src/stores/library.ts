import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

export interface FileItem {
  id: number;
  name: string;
  kind: number; // 0 zdjęcie, 1 wideo
  blurhash: string | null;
  thumb: string | null; // hex hash miniaturki
  duration: number | null;
  rating: number;
  protected_album: number | null;
  locked: boolean;
}

export interface FileInfo {
  id: number;
  path: string;
  name: string;
  kind: number;
  size: number;
  width: number | null;
  height: number | null;
  duration: number | null;
  taken_at: number | null;
}

export type Sort = "date_desc" | "date_asc" | "name" | "size_desc";

/** Filtry listy — odzwierciedla catalog::ListQuery po stronie Rust. */
export interface ListQuery {
  parent: string | null;
  recursive: boolean;
  query: string | null;
  kind: number | null;
  rating_min: number;
  tag_id: number | null;
  album_id: number | null;
  date_from: number | null;
  date_to: number | null;
  sort: Sort;
}

const DEFAULT_QUERY: ListQuery = {
  parent: "",
  recursive: true,
  query: null,
  kind: null,
  rating_min: 0,
  tag_id: null,
  album_id: null,
  date_from: null,
  date_to: null,
  sort: "date_desc",
};

export const PAGE = 500;

interface LibraryState {
  q: ListQuery;
  total: number;
  pages: Record<number, FileItem[]>;
  loadingPages: Set<number>;
  cellSize: number;
  folders: [string, number][];
  timeline: [string, number][]; // histogram miesięcy dla scrubbera daty
  indexing: { pending: number; total: number } | null;
  lightbox: number | null;
  selection: Set<number>;
  lastClicked: number | null; // indeks do zaznaczania Shift+klik

  setQuery: (patch: Partial<ListQuery>) => void;
  resetQuery: (patch?: Partial<ListQuery>) => void;
  setFolder: (parent: string) => void;
  setCellSize: (size: number) => void;
  refresh: () => Promise<void>;
  ensurePage: (page: number) => void;
  itemAt: (index: number) => FileItem | undefined;
  openLightbox: (index: number) => void;
  closeLightbox: () => void;
  setLightbox: (index: number) => void;
  toggleSelect: (id: number, index: number, shift: boolean) => void;
  clearSelection: () => void;
  loadFolders: () => Promise<void>;
  setIndexing: (p: { pending: number; total: number }) => void;
}

export const useLibrary = create<LibraryState>((set, get) => ({
  q: { ...DEFAULT_QUERY },
  total: 0,
  pages: {},
  loadingPages: new Set(),
  cellSize: 160,
  folders: [],
  timeline: [],
  indexing: null,
  lightbox: null,
  selection: new Set(),
  lastClicked: null,

  setQuery: (patch) => {
    set((s) => ({
      q: { ...s.q, ...patch },
      pages: {},
      total: 0,
      lightbox: null,
      selection: new Set(),
      lastClicked: null,
    }));
    get().refresh();
  },
  resetQuery: (patch) => {
    set({
      q: { ...DEFAULT_QUERY, ...patch },
      pages: {},
      total: 0,
      lightbox: null,
      selection: new Set(),
      lastClicked: null,
    });
    get().refresh();
  },
  setFolder: (parent) => {
    invoke("set_focus_folder", { parent }).catch(() => {});
    get().resetQuery({ parent });
  },
  setCellSize: (cellSize) => set({ cellSize }),

  refresh: async () => {
    const { q } = get();
    const total = await invoke<number>("count_files", { q });
    // filtr mógł się zmienić w trakcie zapytania
    if (get().q !== q) return;
    // stale-while-revalidate: stare strony zostają widoczne, świeże dane
    // podmieniają je w tle — bez czarnego mrugnięcia całej siatki
    set({ total });
    get().loadFolders();
    // histogram daty dla scrubbera — tylko przy sortowaniu po dacie
    if (q.sort === "date_desc" || q.sort === "date_asc") {
      invoke<[string, number][]>("timeline_histogram", { q }).then((timeline) => {
        if (get().q === q) set({ timeline });
      });
    } else {
      set({ timeline: [] });
    }
    const loaded = Object.keys(get().pages).map(Number);
    if (loaded.length > 16) {
      // dawno przescrollowane strony wyrzucamy zamiast odświeżać wszystkie
      set({ pages: {}, loadingPages: new Set() });
      return;
    }
    for (const page of loaded) {
      invoke<FileItem[]>("list_files", { q, offset: page * PAGE, limit: PAGE }).then(
        (items) => {
          if (get().q !== q) return;
          set((s) => ({ pages: { ...s.pages, [page]: items } }));
        },
      );
    }
  },

  ensurePage: (page) => {
    const { pages, loadingPages, q } = get();
    if (page < 0 || pages[page] || loadingPages.has(page)) return;
    set({ loadingPages: new Set(loadingPages).add(page) });
    invoke<FileItem[]>("list_files", { q, offset: page * PAGE, limit: PAGE })
      .then((items) => {
        if (get().q !== q) return;
        set((s) => ({ pages: { ...s.pages, [page]: items } }));
      })
      .finally(() => {
        const next = new Set(get().loadingPages);
        next.delete(page);
        set({ loadingPages: next });
      });
  },

  itemAt: (index) => {
    const { pages } = get();
    return pages[Math.floor(index / PAGE)]?.[index % PAGE];
  },

  openLightbox: (index) => set({ lightbox: index }),
  closeLightbox: () => set({ lightbox: null }),
  setLightbox: (index) => {
    const { total } = get();
    if (index >= 0 && index < total) set({ lightbox: index });
  },

  toggleSelect: (id, index, shift) => {
    const { selection, lastClicked, itemAt } = get();
    const next = new Set(selection);
    if (shift && lastClicked !== null) {
      const [a, b] = [Math.min(lastClicked, index), Math.max(lastClicked, index)];
      for (let i = a; i <= b; i++) {
        const it = itemAt(i);
        if (it) next.add(it.id);
      }
    } else if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    set({ selection: next, lastClicked: index });
  },
  clearSelection: () => set({ selection: new Set(), lastClicked: null }),

  loadFolders: async () => {
    const folders = await invoke<[string, number][]>("list_folders");
    set({ folders });
  },

  setIndexing: (p) => set({ indexing: p.pending > 0 ? p : null }),
}));

export function formatDuration(seconds: number): string {
  const s = Math.round(seconds);
  const m = Math.floor(s / 60);
  const h = Math.floor(m / 60);
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m % 60)}:${pad(s % 60)}` : `${m}:${pad(s % 60)}`;
}

export function formatSize(bytes: number): string {
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
