import { useEffect, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { formatSize, useLibrary } from "../stores/library";
import { useApp } from "../stores/app";

interface ImportPlan {
  total: number;
  total_size: number;
  photos: number;
  videos: number;
  tree: [string, number][];
}

interface ImportProgress {
  done: number;
  total: number;
  imported: number;
  duplicates: number;
  errors: number;
}

interface PendingItem {
  id: number;
  src: string;
  dst: string;
  size: number;
  dup_id: number;
  dup_path: string;
  dup_name: string;
  dup_size: number;
  dup_thumb: string | null;
  dup_taken_at: number | null;
  kind: number;
}

const TEMPLATES: [string, string][] = [
  ["{rok}/{miesiac}", "Rok / miesiąc (2024/07)"],
  ["{rok}/{rok}-{miesiac}-{dzien}", "Rok / pełna data (2024/2024-07-15)"],
  ["{rok}/{typ}/{miesiac}", "Rok / typ / miesiąc"],
  ["{typ}/{rok}/{miesiac}", "Typ / rok / miesiąc"],
  ["{folder}", "Zachowaj folder źródłowy"],
];

function TemplatePicker({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
}) {
  const preset = TEMPLATES.some(([t]) => t === value);
  return (
    <label className="block">
      <span className="text-[12px] text-ink-dim">{label}</span>
      <div className="mt-1 flex gap-2">
        <select
          value={preset ? value : "custom"}
          onChange={(e) => {
            if (e.target.value !== "custom") onChange(e.target.value);
          }}
          className="rounded-md border border-edge bg-raised px-2 py-1.5 text-[13px] outline-none focus:border-accent"
        >
          {TEMPLATES.map(([t, desc]) => (
            <option key={t} value={t}>
              {desc}
            </option>
          ))}
          <option value="custom">Własny…</option>
        </select>
        <input
          value={value}
          onChange={(e) => onChange(e.target.value)}
          spellCheck={false}
          className="flex-1 rounded-md border border-edge bg-app px-2 py-1.5 font-mono text-[12px] text-ink-dim outline-none focus:border-accent"
          title="Tokeny: {rok} {miesiac} {dzien} {typ} {folder}"
        />
      </div>
    </label>
  );
}

function PendingReview({ onDone }: { onDone: () => void }) {
  const [items, setItems] = useState<PendingItem[]>([]);
  const [busy, setBusy] = useState(false);

  const load = () =>
    invoke<PendingItem[]>("list_import_pending").then((rows) => {
      setItems(rows);
      if (rows.length === 0) onDone();
    });
  useEffect(() => {
    load();
  }, []);

  async function resolve(ids: number[], action: string) {
    setBusy(true);
    try {
      await invoke("resolve_import_pending", { ids, action });
    } finally {
      setBusy(false);
      load();
    }
  }

  if (items.length === 0) return null;
  const allIds = items.map((i) => i.id);

  return (
    <div className="mx-auto max-w-4xl p-6">
      <div className="mb-4 flex items-center gap-3">
        <h2 className="text-lg font-semibold">
          Duplikaty z importu ({items.length})
        </h2>
        <div className="ml-auto flex gap-2 text-[13px]">
          <button
            disabled={busy}
            onClick={() => resolve(allIds, "import")}
            className="rounded-md border border-edge bg-raised px-3 py-1.5 hover:border-ink-faint disabled:opacity-50"
          >
            Importuj wszystkie
          </button>
          <button
            disabled={busy}
            onClick={() => resolve(allIds, "skip")}
            className="rounded-md border border-edge bg-raised px-3 py-1.5 hover:border-ink-faint disabled:opacity-50"
          >
            Pomiń wszystkie
          </button>
          <button
            disabled={busy}
            onClick={() => resolve(allIds, "delete_source")}
            className="rounded-md border border-danger/40 px-3 py-1.5 text-danger hover:bg-danger/10 disabled:opacity-50"
          >
            Usuń wszystkie ze źródła
          </button>
        </div>
      </div>
      <p className="mb-4 text-[13px] text-ink-dim">
        Te pliki mają identyczną treść jak pliki już obecne w bibliotece. Nic nie
        zostało skopiowane — zdecyduj, co z nimi zrobić. „Usuń ze źródła" przenosi
        plik źródłowy do kosza systemowego.
      </p>
      <div className="space-y-3">
        {items.map((item) => (
          <div
            key={item.id}
            className="flex items-center gap-4 rounded-lg border border-edge bg-surface p-3"
          >
            <div className="flex gap-2">
              <figure className="w-28 text-center">
                <img
                  src={convertFileSrc(`pending/${item.id}`, "media")}
                  className="h-24 w-28 rounded-[4px] object-cover"
                />
                <figcaption className="mt-1 text-[10px] uppercase tracking-wide text-ink-faint">
                  Nowy
                </figcaption>
              </figure>
              <figure className="w-28 text-center">
                {item.dup_thumb ? (
                  <img
                    src={convertFileSrc(item.dup_thumb, "thumb")}
                    className="h-24 w-28 rounded-[4px] object-cover"
                  />
                ) : (
                  <div className="flex h-24 w-28 items-center justify-center rounded-[4px] bg-raised text-2xl opacity-40">
                    {item.kind === 1 ? "🎬" : "🖼"}
                  </div>
                )}
                <figcaption className="mt-1 text-[10px] uppercase tracking-wide text-ink-faint">
                  W bibliotece
                </figcaption>
              </figure>
            </div>
            <div className="min-w-0 flex-1 font-mono text-[11px] leading-5 text-ink-dim">
              <div className="truncate" title={item.src}>
                {item.src}
              </div>
              <div className="truncate text-ink-faint" title={item.dup_path}>
                = {item.dup_path}
              </div>
              <div className="text-ink-faint">{formatSize(item.size)}</div>
            </div>
            <div className="flex shrink-0 flex-col gap-1.5 text-[12px]">
              <button
                disabled={busy}
                onClick={() => resolve([item.id], "import")}
                className="rounded-md bg-accent px-3 py-1 text-white hover:bg-accent-hover disabled:opacity-50"
              >
                Importuj
              </button>
              <button
                disabled={busy}
                onClick={() => resolve([item.id], "skip")}
                className="rounded-md border border-edge px-3 py-1 hover:border-ink-faint disabled:opacity-50"
              >
                Pomiń
              </button>
              <button
                disabled={busy}
                onClick={() => resolve([item.id], "delete_source")}
                className="rounded-md border border-danger/40 px-3 py-1 text-danger hover:bg-danger/10 disabled:opacity-50"
              >
                Usuń ze źródła
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

export function ImportView() {
  const importSource = useApp((s) => s.importSource);
  const [source, setSource] = useState(importSource ?? "");
  const [photoTemplate, setPhotoTemplate] = useState("{rok}/{miesiac}");
  const [videoTemplate, setVideoTemplate] = useState("{rok}/{miesiac}");
  const [separate, setSeparate] = useState(false);
  const [plan, setPlan] = useState<ImportPlan | null>(null);
  const [planning, setPlanning] = useState(false);
  const [progress, setProgress] = useState<ImportProgress | null>(null);
  const [finished, setFinished] = useState<ImportProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reviewing, setReviewing] = useState(false);
  const refresh = useLibrary((s) => s.refresh);

  const videoTpl = separate ? videoTemplate : photoTemplate;

  useEffect(() => {
    const un1 = listen<ImportProgress>("import-progress", (e) => setProgress(e.payload));
    const un2 = listen<ImportProgress>("import-done", (e) => {
      setProgress(null);
      setFinished(e.payload);
      refresh();
      if (e.payload.duplicates > 0) setReviewing(true);
    });
    return () => {
      un1.then((f) => f());
      un2.then((f) => f());
    };
  }, [refresh]);

  // podgląd po wybraniu źródła / zmianie szablonów
  useEffect(() => {
    if (!source) return;
    setPlanning(true);
    setError(null);
    const t = setTimeout(() => {
      invoke<ImportPlan>("import_plan", {
        source,
        photoTemplate,
        videoTemplate: videoTpl,
      })
        .then(setPlan)
        .catch((e) => setError(String(e)))
        .finally(() => setPlanning(false));
    }, 400);
    return () => clearTimeout(t);
  }, [source, photoTemplate, videoTpl]);

  if (reviewing) {
    return <PendingReview onDone={() => setReviewing(false)} />;
  }

  return (
    <div className="mx-auto max-w-2xl p-6">
      <h2 className="text-lg font-semibold tracking-tight">Import plików</h2>
      <p className="mt-1 text-[13px] text-ink-dim">
        Kopiuje zdjęcia i filmy do biblioteki według wybranego schematu. Każdy
        plik jest weryfikowany hashem po skopiowaniu, a duplikaty trafiają do
        poczekalni zamiast do biblioteki.
      </p>

      <section className="mt-5 space-y-4 rounded-lg border border-edge bg-surface p-5">
        <div className="flex items-center gap-3">
          <code className="flex-1 truncate rounded-md border border-edge bg-app px-3 py-2 font-mono text-[12px] text-ink-dim">
            {source || "— wybierz folder źródłowy —"}
          </code>
          <button
            onClick={async () => {
              const p = await open({ directory: true, title: "Folder źródłowy" });
              if (typeof p === "string") setSource(p);
            }}
            className="shrink-0 rounded-md border border-edge bg-raised px-4 py-2 text-[13px] hover:border-ink-faint"
          >
            Wybierz źródło
          </button>
        </div>

        <TemplatePicker
          label={separate ? "Schemat dla zdjęć" : "Schemat organizacji"}
          value={photoTemplate}
          onChange={setPhotoTemplate}
        />
        <label className="flex cursor-pointer items-center gap-2 text-[13px] text-ink-dim">
          <input
            type="checkbox"
            checked={separate}
            onChange={(e) => setSeparate(e.target.checked)}
            className="accent-(--color-accent)"
          />
          Osobny schemat dla wideo
        </label>
        {separate && (
          <TemplatePicker
            label="Schemat dla wideo"
            value={videoTemplate}
            onChange={setVideoTemplate}
          />
        )}
      </section>

      {error && <p className="mt-4 text-sm text-danger">{error}</p>}

      {plan && !progress && (
        <section className="mt-4 rounded-lg border border-edge bg-surface p-5">
          <div className="flex gap-6 text-[13px]">
            <span>
              <b>{plan.total.toLocaleString("pl")}</b> plików
            </span>
            <span>{plan.photos.toLocaleString("pl")} zdjęć</span>
            <span>{plan.videos.toLocaleString("pl")} wideo</span>
            <span>{formatSize(plan.total_size)}</span>
          </div>
          {plan.tree.length > 0 && (
            <div className="mt-3 max-h-40 overflow-y-auto rounded-md border border-edge bg-app p-2 font-mono text-[11px] leading-5 text-ink-dim">
              {plan.tree.slice(0, 50).map(([dir, n]) => (
                <div key={dir}>
                  {dir || "(katalog główny)"} — {n}
                </div>
              ))}
              {plan.tree.length > 50 && (
                <div className="text-ink-faint">… i {plan.tree.length - 50} innych</div>
              )}
            </div>
          )}
          <button
            disabled={planning || plan.total === 0}
            onClick={() => {
              setFinished(null);
              setProgress({ done: 0, total: plan.total, imported: 0, duplicates: 0, errors: 0 });
              invoke("import_run", { source, photoTemplate, videoTemplate: videoTpl });
            }}
            className="mt-4 rounded-md bg-accent px-5 py-2 text-sm font-medium text-white hover:bg-accent-hover disabled:opacity-50"
          >
            Rozpocznij import
          </button>
        </section>
      )}

      {progress && (
        <section className="mt-4 rounded-lg border border-edge bg-surface p-5">
          <div className="mb-2 flex justify-between text-[13px]">
            <span>Importowanie…</span>
            <span className="text-ink-dim">
              {progress.done} / {progress.total}
            </span>
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-raised">
            <div
              className="h-full bg-accent transition-[width] duration-300"
              style={{ width: `${(100 * progress.done) / Math.max(1, progress.total)}%` }}
            />
          </div>
          <div className="mt-2 flex gap-4 text-[12px] text-ink-dim">
            <span>✓ {progress.imported} zaimportowane</span>
            <span>⧉ {progress.duplicates} duplikaty</span>
            {progress.errors > 0 && (
              <span className="text-danger">⚠ {progress.errors} błędy</span>
            )}
          </div>
        </section>
      )}

      {finished && !progress && (
        <section className="mt-4 rounded-lg border border-success/30 bg-success/5 p-5 text-[13px]">
          <b>Import zakończony.</b> Zaimportowano {finished.imported}, duplikatów:{" "}
          {finished.duplicates}, błędów: {finished.errors}.
          {finished.duplicates > 0 && (
            <button
              onClick={() => setReviewing(true)}
              className="ml-3 rounded-md bg-accent px-3 py-1 text-white hover:bg-accent-hover"
            >
              Przejrzyj duplikaty
            </button>
          )}
        </section>
      )}
    </div>
  );
}
