import { create, type StoreApi } from "zustand";
import { invoke } from "@tauri-apps/api/core";

export type View =
  | "all"
  | "timeline"
  | "folders"
  | "albums"
  | "tags"
  | "duplicates"
  | "import"
  | "trash"
  | "settings";

interface Settings {
  library_path: string | null;
}

export interface Toast {
  id: number;
  text: string;
  action?: { label: string; run: () => void };
}

/** Postęp aktywnego zadania kopiowania (z eventu backendu `resolve-progress`). */
export interface CopyProgress {
  done: number;
  total: number;
  ok: number;
  errors: number;
}

export interface CopyActive extends CopyProgress {
  label: string;
}

interface AppState {
  loaded: boolean;
  libraryPath: string | null;
  view: View;
  importSource: string | null; // źródło wstępnie ustawione (np. wykryty nośnik)
  toasts: Toast[];
  unlockRequest: number | null; // id albumu, dla którego pokazać prompt hasła
  // globalna kolejka kopiowania do biblioteki — żyje poza widokiem importu,
  // żeby można było szykować kolejny import, który się dokolejkuje
  copyActive: CopyActive | null;
  copyQueue: { ids: number[]; label: string }[];
  enqueueCopy: (ids: number[], label: string) => void;
  copyProgress: (p: CopyProgress) => void;
  copyDone: () => void;
  setView: (view: View) => void;
  setImportSource: (src: string | null) => void;
  requestUnlock: (albumId: number | null) => void;
  toast: (text: string, action?: Toast["action"]) => void;
  dismissToast: (id: number) => void;
  loadSettings: () => Promise<void>;
  setLibraryPath: (path: string) => Promise<void>;
}

let toastId = 0;

// uruchamia następne zadanie kopiowania, jeśli żadne nie jest aktywne. Front
// serializuje wywołania (jedno na raz) — backend woła resolve_import_pending
// w wątku i sygnalizuje zakończenie eventem resolve-done.
function drainCopy(set: StoreApi<AppState>["setState"], get: StoreApi<AppState>["getState"]) {
  const s = get();
  if (s.copyActive || s.copyQueue.length === 0) return;
  const [job, ...rest] = s.copyQueue;
  set({
    copyQueue: rest,
    copyActive: { label: job.label, total: job.ids.length, done: 0, ok: 0, errors: 0 },
  });
  invoke("resolve_import_pending", { ids: job.ids, action: "import" });
}

export const useApp = create<AppState>((set, get) => ({
  loaded: false,
  libraryPath: null,
  view: "all",
  importSource: null,
  toasts: [],
  unlockRequest: null,
  copyActive: null,
  copyQueue: [],
  enqueueCopy: (ids, label) => {
    if (ids.length === 0) return;
    set((s) => ({ copyQueue: [...s.copyQueue, { ids, label }] }));
    drainCopy(set, get);
  },
  copyProgress: (p) =>
    set((s) => (s.copyActive ? { copyActive: { ...s.copyActive, ...p } } : {})),
  copyDone: () => {
    set({ copyActive: null });
    drainCopy(set, get);
  },
  setView: (view) => set({ view }),
  setImportSource: (importSource) => set({ importSource }),
  requestUnlock: (unlockRequest) => set({ unlockRequest }),
  toast: (text, action) => {
    const id = ++toastId;
    set((s) => ({ toasts: [...s.toasts, { id, text, action }] }));
    window.setTimeout(
      () => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
      action ? 15000 : 5000,
    );
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
  loadSettings: async () => {
    const settings = await invoke<Settings>("get_settings");
    set({ libraryPath: settings.library_path, loaded: true });
  },
  setLibraryPath: async (path) => {
    await invoke("set_library_path", { path });
    set({ libraryPath: path });
  },
}));
