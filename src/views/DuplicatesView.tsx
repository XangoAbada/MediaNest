import { useMemo, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { EmptyState } from "../components/EmptyState";
import { Blurhash } from "../components/Blurhash";
import { formatSize, useLibrary } from "../stores/library";
import { useApp } from "../stores/app";

interface DupMember {
  id: number;
  path: string;
  name: string;
  kind: number;
  size: number;
  width: number | null;
  height: number | null;
  taken_at: number | null;
  thumb: string | null;
  blurhash: string | null;
}

interface DupGroup {
  kind: "exact" | "similar" | "burst";
  members: DupMember[];
}

// stabilny klucz grupy — przetrwa chirurgiczne usuwanie członków, więc
// zaznaczenia w nietkniętych grupach nie gubią się przy re-renderze
type KeyedGroup = DupGroup & { _key: string };

const SECTIONS = [
  ["exact", "Identyczne"],
  ["similar", "Podobne"],
  ["burst", "Serie"],
] as const;

const isCopy = (m: DupMember) => /copy/i.test(m.name);

/// Najlepszy plik w grupie: bez „copy" w nazwie > rozdzielczość > rozmiar > najstarsza data.
function bestOf(members: DupMember[]): number {
  let best = members[0];
  for (const m of members) {
    const copyM = isCopy(m);
    const copyB = isCopy(best);
    if (copyM !== copyB) {
      // nie-kopia zawsze wygrywa z kopią, niezależnie od jakości
      if (!copyM) best = m;
      continue;
    }
    // równy status „copy" → dotychczasowa logika jakości
    const resM = (m.width ?? 0) * (m.height ?? 0);
    const resB = (best.width ?? 0) * (best.height ?? 0);
    if (
      resM > resB ||
      (resM === resB &&
        (m.size > best.size ||
          (m.size === best.size &&
            (m.taken_at ?? Infinity) < (best.taken_at ?? Infinity))))
    ) {
      best = m;
    }
  }
  return best.id;
}

function GroupCard({
  group,
  onTrashed,
}: {
  group: DupGroup;
  onTrashed: (trashedIds: number[]) => void;
}) {
  const best = useMemo(() => bestOf(group.members), [group]);
  // w grupach duplikatów domyślnie zaznaczone wszystko poza najlepszym;
  // serie tylko do przeglądu — bez auto-zaznaczenia
  const [selected, setSelected] = useState<Set<number>>(
    () =>
      new Set(
        group.kind === "burst"
          ? []
          : group.members.filter((m) => m.id !== best).map((m) => m.id),
      ),
  );
  const toast = useApp((s) => s.toast);
  const refresh = useLibrary((s) => s.refresh);

  async function trashSelected() {
    if (selected.size === 0) return;
    const ids = [...selected];
    const n = await invoke<number>("trash_files", { ids });
    if (n === 0) {
      toast("Nie udało się przenieść plików do kosza");
      return;
    }
    toast(`Przeniesiono ${n} plików do kosza`);
    setSelected(new Set());
    refresh();
    onTrashed(ids);
  }

  return (
    <div className="rounded-lg border border-edge bg-surface p-3">
      <div className="space-y-1.5">
        {group.members.map((m) => {
          const sel = selected.has(m.id);
          return (
            <div
              key={m.id}
              onClick={() => {
                const next = new Set(selected);
                if (sel) next.delete(m.id);
                else next.add(m.id);
                setSelected(next);
              }}
              className={`flex cursor-pointer items-center gap-3 rounded-md border px-2 py-1.5 transition-colors ${
                sel
                  ? "border-danger/50 bg-danger/5"
                  : "border-transparent hover:bg-raised"
              }`}
            >
              <input
                type="checkbox"
                checked={sel}
                readOnly
                className="accent-(--color-danger)"
              />
              <div className="relative h-14 w-14 shrink-0 overflow-hidden rounded-[3px] bg-raised">
                {m.blurhash && !m.thumb && (
                  <Blurhash hash={m.blurhash} className="h-full w-full" />
                )}
                {m.thumb && (
                  <img
                    src={convertFileSrc(m.thumb, "thumb")}
                    loading="lazy"
                    className="absolute inset-0 h-full w-full object-cover"
                  />
                )}
              </div>
              <div className="min-w-0 flex-1 font-mono text-[11px] leading-4 text-ink-dim">
                <div className="truncate text-[12px] text-ink">{m.name}</div>
                <div className="truncate">{m.path}</div>
                <div className="text-ink-faint">
                  {m.width && m.height ? `${m.width}×${m.height} · ` : ""}
                  {formatSize(m.size)}
                  {m.taken_at
                    ? ` · ${new Date(m.taken_at * 1000).toLocaleDateString("pl")}`
                    : ""}
                </div>
              </div>
              {m.id === best && group.kind !== "burst" && (
                <span className="shrink-0 rounded bg-success/15 px-2 py-0.5 text-[11px] text-success">
                  zachowaj
                </span>
              )}
            </div>
          );
        })}
      </div>
      {selected.size > 0 && (
        <button
          onClick={trashSelected}
          className="mt-2 rounded-md border border-danger/40 px-3 py-1 text-[12px] text-danger hover:bg-danger/10"
        >
          Usuń zaznaczone do kosza ({selected.size})
        </button>
      )}
    </div>
  );
}

export function DuplicatesView() {
  const [threshold, setThreshold] = useState(6);
  const [groups, setGroups] = useState<KeyedGroup[] | null>(null);
  const [scanning, setScanning] = useState(false);
  const [section, setSection] = useState<"exact" | "similar" | "burst">("exact");
  const indexing = useLibrary((s) => s.indexing);

  async function scan() {
    setScanning(true);
    try {
      const raw = await invoke<DupGroup[]>("dedup_scan", { threshold });
      setGroups(
        raw.map((g, i) => ({ ...g, _key: `${g.kind}-${i}-${g.members[0]?.id}` })),
      );
    } finally {
      setScanning(false);
    }
  }

  // po usunięciu do kosza: wyrzuć te pliki z WSZYSTKICH grup (mogą być w kilku)
  // i usuń grupy, którym zostało mniej niż 2 elementy — bez pełnego re-skanu,
  // dzięki czemu zaznaczenia w pozostałych grupach zostają nietknięte
  function removeTrashed(trashedIds: number[]) {
    const gone = new Set(trashedIds);
    setGroups(
      (prev) =>
        prev
          ?.map((g) => ({ ...g, members: g.members.filter((m) => !gone.has(m.id)) }))
          .filter((g) => g.members.length >= 2) ?? null,
    );
  }

  const visible = groups?.filter((g) => g.kind === section) ?? [];
  const counts = Object.fromEntries(
    SECTIONS.map(([k]) => [k, groups?.filter((g) => g.kind === k).length ?? 0]),
  );

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-4 border-b border-edge bg-surface px-4 py-2.5">
        <button
          onClick={scan}
          disabled={scanning}
          className="rounded-md bg-accent px-4 py-1.5 text-[13px] font-medium text-white hover:bg-accent-hover disabled:opacity-50"
        >
          {scanning ? "Skanowanie…" : groups ? "Skanuj ponownie" : "Skanuj duplikaty"}
        </button>
        <label className="flex items-center gap-2 text-[12px] text-ink-dim">
          Czułość podobieństwa
          <input
            type="range"
            min={0}
            max={7}
            value={threshold}
            onChange={(e) => setThreshold(Number(e.target.value))}
            className="w-28 accent-(--color-accent)"
          />
          <span className="w-4 font-mono">{threshold}</span>
        </label>
        {indexing && (
          <span className="text-[12px] text-warning">
            Indeksowanie trwa — wyniki mogą być niepełne
          </span>
        )}
        {groups && (
          <div className="ml-auto flex gap-1">
            {SECTIONS.map(([key, label]) => (
              <button
                key={key}
                onClick={() => setSection(key)}
                className={`rounded-md px-3 py-1 text-[13px] ${
                  section === key
                    ? "bg-accent/15 text-ink"
                    : "text-ink-dim hover:text-ink"
                }`}
              >
                {label} ({counts[key]})
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {!groups ? (
          <EmptyState
            icon="⧉"
            title="Znajdź duplikaty"
            description="Skan porówna wszystkie pliki po treści (BLAKE3) i podobieństwie wizualnym (pHash). Usuwanie zawsze przez kosz — nic nie ginie bezpowrotnie."
          />
        ) : visible.length === 0 ? (
          <EmptyState icon="✓" title="Brak grup w tej sekcji" />
        ) : (
          <div className="mx-auto max-w-3xl space-y-3 p-4">
            {section === "similar" && (
              <p className="text-[12px] text-ink-faint">
                Grupy zdjęć wizualnie podobnych (przeskalowane kopie, drobne
                edycje). Sprawdź przed usunięciem — podobieństwo to nie
                identyczność.
              </p>
            )}
            {visible.slice(0, 100).map((g) => (
              <GroupCard key={g._key} group={g} onTrashed={removeTrashed} />
            ))}
            {visible.length > 100 && (
              <p className="py-2 text-center text-[12px] text-ink-faint">
                Pokazano 100 z {visible.length} grup — usuń te i skanuj ponownie.
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
