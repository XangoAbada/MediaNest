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
  new_files: number;
  duplicates: number;
  skipped: number;
  errors: number;
  cancelled: boolean;
}

interface PendingItem {
  id: number;
  src: string;
  dst: string;
  size: number;
  kind: number;
  // pola dup_* wypełnione tylko dla duplikatów; dla nowych plików null
  dup_id: number | null;
  dup_path: string | null;
  dup_name: string | null;
  dup_size: number | null;
  dup_thumb: string | null;
  dup_taken_at: number | null;
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

function basename(p: string) {
  return p.split(/[\\/]/).pop() || p;
}

function PendingReview({ onDone }: { onDone: () => void }) {
  const [items, setItems] = useState<PendingItem[]>([]);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const enqueueCopy = useApp((s) => s.enqueueCopy);

  // domyślnie zaznaczone są nowe pliki, duplikaty odznaczone
  const load = () =>
    invoke<PendingItem[]>("list_import_pending").then((rows) => {
      setItems(rows);
      setSelected(new Set(rows.filter((r) => r.dup_id == null).map((r) => r.id)));
      if (rows.length === 0) onDone();
    });
  useEffect(() => {
    load();
  }, []);

  // kopiowanie trafia do GLOBALNEJ kolejki (panel widoczny w każdym widoku) —
  // po dodaniu opuszczamy przegląd, by można było przygotować kolejny import
  function addToLibrary(ids: number[]) {
    if (ids.length === 0) return;
    enqueueCopy(ids, `${ids.length} plików`);
    onDone();
  }

  // szybkie akcje (pomiń / usuń ze źródła) — synchronicznie, lokalnie
  async function resolve(ids: number[], action: string) {
    if (ids.length === 0) return;
    setBusy(true);
    try {
      await invoke("resolve_import_pending", { ids, action });
    } finally {
      setBusy(false);
      load();
    }
  }

  const toggle = (id: number) =>
    setSelected((s) => {
      const n = new Set(s);
      n.has(id) ? n.delete(id) : n.add(id);
      return n;
    });
  const toggleGroup = (ids: number[], on: boolean) =>
    setSelected((s) => {
      const n = new Set(s);
      ids.forEach((id) => (on ? n.add(id) : n.delete(id)));
      return n;
    });

  if (items.length === 0) return null;
  const news = items.filter((i) => i.dup_id == null);
  const dups = items.filter((i) => i.dup_id != null);
  const selIds = [...selected].filter((id) => items.some((i) => i.id === id));
  const allIds = items.map((i) => i.id);
  const newAllOn = news.length > 0 && news.every((i) => selected.has(i.id));

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-4xl p-6">
        <h2 className="text-lg font-semibold">Przegląd importu ({items.length})</h2>
        <p className="mt-1 mb-4 text-[13px] text-ink-dim">
          Nic nie zostało jeszcze skopiowane. Zaznacz pliki, które mają trafić do
          biblioteki. „Usuń ze źródła" przenosi plik źródłowy do kosza systemowego.
        </p>

        {/* pasek akcji na zaznaczonych */}
        <div className="sticky top-0 z-10 mb-4 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-lg border border-edge bg-surface/95 px-4 py-2.5 backdrop-blur">
          <span className="text-[13px] text-ink-dim">
            Zaznaczono <b className="text-ink">{selIds.length}</b> z {items.length}
          </span>
          <div className="ml-auto flex flex-wrap gap-2 text-[13px]">
            <button
              disabled={busy || selIds.length === 0}
              onClick={() => addToLibrary(selIds)}
              className="rounded-md bg-accent px-3 py-1.5 font-medium text-white hover:bg-accent-hover disabled:opacity-50"
            >
              Dodaj zaznaczone do biblioteki
            </button>
            <button
              disabled={busy || selIds.length === 0}
              onClick={() => resolve(selIds, "skip")}
              className="rounded-md border border-edge bg-raised px-3 py-1.5 hover:border-ink-faint disabled:opacity-50"
            >
              Pomiń zaznaczone
            </button>
            <button
              disabled={busy || selIds.length === 0}
              onClick={() => resolve(selIds, "delete_source")}
              className="rounded-md border border-danger/40 px-3 py-1.5 text-danger hover:bg-danger/10 disabled:opacity-50"
            >
              Usuń zaznaczone ze źródła
            </button>
            <button
              disabled={busy}
              onClick={() => resolve(allIds, "skip")}
              title="Usuwa całą listę z poczekalni (plików nie kopiuje ani nie kasuje)"
              className="rounded-md border border-edge px-3 py-1.5 text-ink-dim hover:border-ink-faint disabled:opacity-50"
            >
              Wyczyść poczekalnię
            </button>
          </div>
        </div>


      {/* sekcja: nowe pliki (bez duplikatu) */}
      {news.length > 0 && (
        <section className="mb-6">
          <div className="mb-2 flex items-center gap-3">
            <h3 className="text-[13px] font-semibold uppercase tracking-wide text-ink-dim">
              Do importu — nowe ({news.length})
            </h3>
            <label className="flex cursor-pointer items-center gap-1.5 text-[12px] text-ink-dim">
              <input
                type="checkbox"
                checked={newAllOn}
                onChange={(e) => toggleGroup(news.map((i) => i.id), e.target.checked)}
                className="accent-(--color-accent)"
              />
              Zaznacz wszystkie
            </label>
          </div>
          {/* ponytail: bez windowingu; dodać wirtualizację, gdy duże importy zaczną zamulać */}
          <div className="grid grid-cols-[repeat(auto-fill,minmax(120px,1fr))] gap-2">
            {news.map((item) => (
              <label
                key={item.id}
                className="group relative cursor-pointer overflow-hidden rounded-lg border border-edge bg-surface"
              >
                <input
                  type="checkbox"
                  checked={selected.has(item.id)}
                  onChange={() => toggle(item.id)}
                  className="absolute left-1.5 top-1.5 z-10 accent-(--color-accent)"
                />
                {item.kind === 1 ? (
                  <div
                    className={`flex h-28 w-full items-center justify-center bg-raised text-2xl transition-opacity ${
                      selected.has(item.id) ? "" : "opacity-40"
                    }`}
                  >
                    🎬
                  </div>
                ) : (
                  <img
                    src={convertFileSrc(`pending/${item.id}`, "media")}
                    loading="lazy"
                    decoding="async"
                    className={`h-28 w-full object-cover transition-opacity ${
                      selected.has(item.id) ? "" : "opacity-40"
                    }`}
                  />
                )}
                <div
                  className="truncate px-2 py-1 font-mono text-[10px] text-ink-dim"
                  title={item.src}
                >
                  {basename(item.src)}
                </div>
              </label>
            ))}
          </div>
        </section>
      )}

      {/* sekcja: duplikaty (identyczna treść już w bibliotece) */}
      {dups.length > 0 && (
        <section>
          <h3 className="mb-2 text-[13px] font-semibold uppercase tracking-wide text-ink-dim">
            Duplikaty ({dups.length}) — identyczna treść już w bibliotece
          </h3>
          <div className="space-y-3">
            {dups.map((item) => (
              <label
                key={item.id}
                className="flex cursor-pointer items-center gap-4 rounded-lg border border-edge bg-surface p-3"
              >
                <input
                  type="checkbox"
                  checked={selected.has(item.id)}
                  onChange={() => toggle(item.id)}
                  className="accent-(--color-accent)"
                />
                <div className="flex gap-2">
                  <figure className="w-28 text-center">
                    {item.kind === 1 ? (
                      <div className="flex h-24 w-28 items-center justify-center rounded-[4px] bg-raised text-2xl opacity-40">
                        🎬
                      </div>
                    ) : (
                      <img
                        src={convertFileSrc(`pending/${item.id}`, "media")}
                        loading="lazy"
                        decoding="async"
                        className="h-24 w-28 rounded-[4px] object-cover"
                      />
                    )}
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
                  <div
                    className="truncate text-ink-faint"
                    title={item.dup_path ?? ""}
                  >
                    = {item.dup_path}
                  </div>
                  <div className="text-ink-faint">{formatSize(item.size)}</div>
                </div>
              </label>
            ))}
          </div>
        </section>
      )}
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
  const [stopping, setStopping] = useState(false);
  const [pendingCount, setPendingCount] = useState(0);
  const refresh = useLibrary((s) => s.refresh);

  const videoTpl = separate ? videoTemplate : photoTemplate;

  // poczekalnia bywa niepusta z poprzedniego (być może przerwanego) skanu —
  // pokazujemy do niej wejście, żeby wczytane pliki nie zostały „uwięzione"
  useEffect(() => {
    if (!reviewing) {
      invoke<PendingItem[]>("list_import_pending").then((r) => setPendingCount(r.length));
    }
  }, [reviewing]);

  useEffect(() => {
    const un1 = listen<ImportProgress>("import-progress", (e) => setProgress(e.payload));
    const un2 = listen<ImportProgress>("import-done", (e) => {
      setProgress(null);
      setFinished(e.payload);
      refresh();
      // skan niczego nie kopiuje — zawsze przechodzimy do przeglądu wyboru
      if (e.payload.new_files > 0 || e.payload.duplicates > 0) setReviewing(true);
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
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-2xl p-6">
      <h2 className="text-lg font-semibold tracking-tight">Import plików</h2>
      <p className="mt-1 text-[13px] text-ink-dim">
        Skanuje zdjęcia i filmy oraz wykrywa duplikaty, ale nic nie kopiuje — po
        skanie wybierasz zaznaczeniem, co trafi do biblioteki. Wybrane pliki są
        układane według schematu i weryfikowane hashem po skopiowaniu.
      </p>

      {pendingCount > 0 && !progress && (
        <div className="mt-4 flex items-center gap-3 rounded-lg border border-accent/40 bg-accent/5 px-4 py-3 text-[13px]">
          <span>
            W poczekalni czeka <b>{pendingCount}</b> wczytanych plików na decyzję.
          </span>
          <button
            onClick={() => setReviewing(true)}
            className="ml-auto shrink-0 rounded-md bg-accent px-3 py-1 text-white hover:bg-accent-hover"
          >
            Przejrzyj poczekalnię
          </button>
        </div>
      )}

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
              setStopping(false);
              setProgress({
                done: 0,
                total: plan.total,
                new_files: 0,
                duplicates: 0,
                skipped: 0,
                errors: 0,
                cancelled: false,
              });
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
          <div className="mb-2 flex items-center justify-between text-[13px]">
            <span>Skanowanie…</span>
            <div className="flex items-center gap-3">
              <span className="text-ink-dim">
                {progress.done} / {progress.total}
              </span>
              <button
                disabled={stopping}
                onClick={() => {
                  setStopping(true);
                  invoke("cancel_import");
                }}
                className="rounded-md border border-danger/40 px-3 py-1 text-[12px] text-danger hover:bg-danger/10 disabled:opacity-50"
              >
                {stopping ? "Zatrzymywanie…" : "Zatrzymaj"}
              </button>
            </div>
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-raised">
            <div
              className="h-full bg-accent transition-[width] duration-300"
              style={{ width: `${(100 * progress.done) / Math.max(1, progress.total)}%` }}
            />
          </div>
          <div className="mt-2 flex gap-4 text-[12px] text-ink-dim">
            <span>✓ {progress.new_files} nowych</span>
            <span>⧉ {progress.duplicates} duplikaty</span>
            {progress.skipped > 0 && <span>⏭ {progress.skipped} pominięto</span>}
            {progress.errors > 0 && (
              <span className="text-danger">⚠ {progress.errors} błędy</span>
            )}
          </div>
        </section>
      )}

      {finished && !progress && (
        <section className="mt-4 rounded-lg border border-success/30 bg-success/5 p-5 text-[13px]">
          <b>{finished.cancelled ? "Skan zatrzymany." : "Skan zakończony."}</b> Nowych
          plików: {finished.new_files}, duplikatów: {finished.duplicates}
          {finished.skipped > 0 && <>, pominięto: {finished.skipped}</>}, błędów:{" "}
          {finished.errors}.
          {finished.new_files + finished.duplicates > 0 && (
            <button
              onClick={() => setReviewing(true)}
              className="ml-3 rounded-md bg-accent px-3 py-1 text-white hover:bg-accent-hover"
            >
              Przejrzyj i wybierz
            </button>
          )}
        </section>
      )}
      </div>
    </div>
  );
}
