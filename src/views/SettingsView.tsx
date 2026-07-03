import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useApp } from "../stores/app";
import { formatSize, useLibrary } from "../stores/library";

interface LibraryStats {
  files: number;
  photos: number;
  videos: number;
  total_size: number;
  missing: number;
  protected: number;
  by_year: [string, number, number][];
}

function Stats() {
  const [stats, setStats] = useState<LibraryStats | null>(null);
  const toast = useApp((s) => s.toast);
  const refresh = useLibrary((s) => s.refresh);

  const load = () => invoke<LibraryStats>("library_stats").then(setStats);
  useEffect(() => {
    load();
  }, []);

  if (!stats) return null;
  const maxCount = Math.max(1, ...stats.by_year.map(([, n]) => n));

  return (
    <>
      <section className="mt-4 rounded-lg border border-edge bg-surface p-5">
        <h3 className="text-sm font-medium">Statystyki biblioteki</h3>
        <div className="mt-3 flex gap-6 text-[13px] text-ink-dim">
          <span>
            <b className="text-ink">{stats.files.toLocaleString("pl")}</b> plików
          </span>
          <span>{stats.photos.toLocaleString("pl")} zdjęć</span>
          <span>{stats.videos.toLocaleString("pl")} wideo</span>
          <span>{formatSize(stats.total_size)}</span>
          {stats.protected > 0 && <span>🔒 {stats.protected} chronionych</span>}
        </div>
        <div className="mt-4 space-y-1.5">
          {stats.by_year.slice(0, 15).map(([year, count, size]) => (
            <div key={year} className="flex items-center gap-2 text-[12px]">
              <span className="w-10 shrink-0 font-mono text-ink-faint">{year}</span>
              <div className="h-3 flex-1 overflow-hidden rounded-sm bg-raised">
                <div
                  className="h-full rounded-sm bg-accent/60"
                  style={{ width: `${(100 * count) / maxCount}%` }}
                />
              </div>
              <span className="w-32 shrink-0 text-right text-ink-faint">
                {count.toLocaleString("pl")} · {formatSize(size)}
              </span>
            </div>
          ))}
        </div>
      </section>

      {stats.missing > 0 && (
        <section className="mt-4 rounded-lg border border-warning/30 bg-warning/5 p-5">
          <h3 className="text-sm font-medium">Zdrowie biblioteki</h3>
          <p className="mt-1 text-[13px] text-ink-dim">
            {stats.missing} wpisów wskazuje na pliki, których nie ma już na dysku.
            Rekoncyliacja odnajdzie przeniesione pliki po treści (hash) i przeniesie
            ich oceny, tagi i albumy; wpisy bez pary zostaną usunięte.
          </p>
          <button
            onClick={async () => {
              const [merged, removed] = await invoke<[number, number]>("reconcile_missing");
              toast(`Scalono ${merged} przeniesionych, usunięto ${removed} martwych wpisów`);
              load();
              refresh();
            }}
            className="mt-3 rounded-md border border-edge bg-raised px-4 py-1.5 text-[13px] hover:border-ink-faint"
          >
            Uruchom rekoncyliację
          </button>
        </section>
      )}
    </>
  );
}

export function SettingsView() {
  const { libraryPath, setLibraryPath } = useApp();
  const [error, setError] = useState<string | null>(null);
  const [theme, setTheme] = useState(
    () => localStorage.getItem("theme") ?? "dark",
  );

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("theme", theme);
  }, [theme]);

  async function changeFolder() {
    setError(null);
    const path = await open({
      directory: true,
      title: "Wybierz nowy folder biblioteki",
      defaultPath: libraryPath ?? undefined,
    });
    if (typeof path !== "string") return;
    try {
      await setLibraryPath(path);
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="mx-auto max-w-2xl overflow-y-auto p-8">
      <h2 className="text-lg font-semibold tracking-tight">Ustawienia</h2>

      <section className="mt-6 rounded-lg border border-edge bg-surface p-5">
        <h3 className="text-sm font-medium">Folder biblioteki</h3>
        <p className="mt-1 text-[13px] text-ink-dim">
          Główny folder ze zdjęciami i filmami. Zmiana folderu uruchomi ponowne
          indeksowanie.
        </p>
        <div className="mt-3 flex items-center gap-3">
          <code className="flex-1 truncate rounded-md border border-edge bg-app px-3 py-2 font-mono text-[12px] text-ink-dim">
            {libraryPath ?? "—"}
          </code>
          <button
            onClick={changeFolder}
            className="shrink-0 rounded-md border border-edge bg-raised px-4 py-2 text-[13px] font-medium transition-colors duration-100 hover:border-ink-faint"
          >
            Zmień folder
          </button>
        </div>
        {error && <p className="mt-2 text-sm text-danger">{error}</p>}
      </section>

      <section className="mt-4 rounded-lg border border-edge bg-surface p-5">
        <h3 className="text-sm font-medium">Wygląd</h3>
        <div className="mt-2 flex gap-2">
          {(["dark", "light"] as const).map((t) => (
            <button
              key={t}
              onClick={() => setTheme(t)}
              className={`rounded-md border px-4 py-1.5 text-[13px] ${
                theme === t
                  ? "border-accent bg-accent/10 text-ink"
                  : "border-edge text-ink-dim hover:border-ink-faint"
              }`}
            >
              {t === "dark" ? "🌙 Ciemny" : "☀ Jasny"}
            </button>
          ))}
        </div>
      </section>

      <Stats />
    </div>
  );
}
