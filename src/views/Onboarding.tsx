import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useApp } from "../stores/app";

export function Onboarding() {
  const setLibraryPath = useApp((s) => s.setLibraryPath);
  const [error, setError] = useState<string | null>(null);

  async function pickFolder() {
    setError(null);
    const path = await open({
      directory: true,
      title: "Wybierz folder biblioteki MediaNest",
    });
    if (typeof path !== "string") return;
    try {
      await setLibraryPath(path);
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="flex h-full items-center justify-center">
      <div className="flex w-[420px] flex-col items-center gap-5 rounded-2xl border border-edge bg-surface p-10 text-center shadow-lg">
        <div className="text-6xl">🪺</div>
        <div>
          <h1 className="text-xl font-semibold tracking-tight">
            Witaj w MediaNest
          </h1>
          <p className="mt-2 text-sm leading-relaxed text-ink-dim">
            Wybierz folder, w którym trzymasz zdjęcia i filmy. MediaNest
            zindeksuje jego zawartość i będzie obserwować zmiany. Folder możesz
            później zmienić w ustawieniach.
          </p>
        </div>
        <button
          onClick={pickFolder}
          className="rounded-md bg-accent px-5 py-2.5 text-sm font-medium text-white transition-colors duration-100 hover:bg-accent-hover"
        >
          Wybierz folder biblioteki
        </button>
        {error && <p className="text-sm text-danger">{error}</p>}
      </div>
    </div>
  );
}
