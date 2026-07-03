import { create } from "zustand";
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

interface AppState {
  loaded: boolean;
  libraryPath: string | null;
  view: View;
  importSource: string | null; // źródło wstępnie ustawione (np. wykryty nośnik)
  toasts: Toast[];
  unlockRequest: number | null; // id albumu, dla którego pokazać prompt hasła
  setView: (view: View) => void;
  setImportSource: (src: string | null) => void;
  requestUnlock: (albumId: number | null) => void;
  toast: (text: string, action?: Toast["action"]) => void;
  dismissToast: (id: number) => void;
  loadSettings: () => Promise<void>;
  setLibraryPath: (path: string) => Promise<void>;
}

let toastId = 0;

export const useApp = create<AppState>((set) => ({
  loaded: false,
  libraryPath: null,
  view: "all",
  importSource: null,
  toasts: [],
  unlockRequest: null,
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
