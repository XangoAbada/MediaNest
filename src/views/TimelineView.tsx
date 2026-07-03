import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { EmptyState } from "../components/EmptyState";
import { Grid } from "../components/Grid";
import { useLibrary } from "../stores/library";

const MONTHS = [
  "styczeń", "luty", "marzec", "kwiecień", "maj", "czerwiec",
  "lipiec", "sierpień", "wrzesień", "październik", "listopad", "grudzień",
];

function monthLabel(ym: string): string {
  const [y, m] = ym.split("-");
  return `${MONTHS[Number(m) - 1]} ${y}`;
}

function monthRange(ym: string): [number, number] {
  const [y, m] = ym.split("-").map(Number);
  const from = Date.UTC(y, m - 1, 1) / 1000;
  const to = Date.UTC(m === 12 ? y + 1 : y, m === 12 ? 0 : m, 1) / 1000;
  return [from, to];
}

export function TimelineView() {
  const [months, setMonths] = useState<[string, number][] | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const { resetQuery } = useLibrary();

  useEffect(() => {
    invoke<[string, number][]>("timeline_months").then(setMonths);
  }, []);

  if (open) {
    return (
      <div className="flex h-full flex-col">
        <div className="flex items-center gap-3 border-b border-edge bg-surface px-4 py-2 text-[13px]">
          <button
            onClick={() => {
              setOpen(null);
              resetQuery();
            }}
            className="text-ink-dim hover:text-ink"
          >
            ← Oś czasu
          </button>
          <span className="font-medium capitalize">{monthLabel(open)}</span>
        </div>
        <div className="min-h-0 flex-1">
          <Grid />
        </div>
      </div>
    );
  }

  if (months && months.length === 0) {
    return <EmptyState icon="📅" title="Brak plików z datami" />;
  }

  // grupowanie po roku
  const byYear = new Map<string, [string, number][]>();
  for (const [ym, count] of months ?? []) {
    const year = ym.slice(0, 4);
    if (!byYear.has(year)) byYear.set(year, []);
    byYear.get(year)!.push([ym, count]);
  }

  return (
    <div className="mx-auto max-w-3xl p-6">
      <h2 className="mb-4 text-lg font-semibold tracking-tight">Oś czasu</h2>
      {[...byYear.entries()].map(([year, list]) => (
        <div key={year} className="mb-5">
          <div className="mb-2 text-[13px] font-semibold text-ink-dim">{year}</div>
          <div className="flex flex-wrap gap-2">
            {list.map(([ym, count]) => (
              <button
                key={ym}
                onClick={() => {
                  setOpen(ym);
                  const [from, to] = monthRange(ym);
                  resetQuery({ parent: null, date_from: from, date_to: to });
                }}
                className="rounded-md border border-edge bg-surface px-3 py-2 text-left transition-colors hover:border-accent"
              >
                <div className="text-[13px] capitalize">{monthLabel(ym)}</div>
                <div className="text-[11px] text-ink-faint">{count} plików</div>
              </button>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
