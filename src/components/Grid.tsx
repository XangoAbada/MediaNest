import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useLibrary, PAGE } from "../stores/library";
import { useApp } from "../stores/app";
import { Thumb } from "./Thumb";
import { EmptyState } from "./EmptyState";
import { DateScrubber } from "./DateScrubber";
import { FolderPicker } from "./FolderPicker";

const GAP = 4;
const HEADER_YEAR = 44;
const HEADER_MONTH = 32;

const MONTHS = [
  "styczeń", "luty", "marzec", "kwiecień", "maj", "czerwiec",
  "lipiec", "sierpień", "wrzesień", "październik", "listopad", "grudzień",
];
const monthName = (ym: string) => MONTHS[Number(ym.split("-")[1]) - 1] ?? ym;
const monthLabel = (ym: string) => `${monthName(ym)} ${ym.slice(0, 4)}`;

type LayoutRow =
  | { type: "year"; year: string }
  | { type: "month"; ym: string | null } // null → sekcja "Bez daty"
  | { type: "cells"; start: number; end: number }; // globalne indeksy plików [start, end)

export function Grid() {
  const { total, pages, ensurePage, cellSize, indexing, openLightbox, selection, toggleSelect } =
    useLibrary();
  const sort = useLibrary((s) => s.q.sort);
  const timeline = useLibrary((s) => s.timeline);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [menu, setMenu] = useState<{ x: number; y: number; ids: number[] } | null>(null);
  const [pickerIds, setPickerIds] = useState<number[] | null>(null);

  // zamknij menu kontekstowe na dowolne kliknięcie / Esc
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setMenu(null);
    window.addEventListener("click", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu]);

  // kontener renderuje się zawsze (EmptyState w środku), więc observer
  // podpina się raz i działa — szerokość znana od pierwszego layoutu
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    setWidth(el.clientWidth);
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const dateSorted = sort === "date_desc" || sort === "date_asc";
  const hasScrubber = dateSorted && timeline.length > 0 && total >= 50;
  // separatory miesięcy/lat tylko przy sortowaniu po dacie i >1 miesiącu
  const sectioned = dateSorted && timeline.length > 1;
  // rezerwa na scrubber po prawej, żeby ostatnia kolumna go nie chowała
  const SCRUB = 36;
  const usable = width - (hasScrubber ? SCRUB : 0);
  const cols = Math.max(1, Math.floor((usable - GAP) / (cellSize + GAP)));
  const rowHeight = cellSize + GAP;

  // sekcjonowany layout o zmiennej wysokości: nagłówki lat/miesięcy + wiersze
  // kafli. Jedno źródło geometrii dla wirtualizera i scrubbera.
  const { layout, rowOffsets, totalHeight, cellRows, sections } = useMemo(() => {
    const layout: LayoutRow[] = [];
    const rowOffsets: number[] = [];
    const cellRows: { first: number; row: number }[] = []; // wiersze kafli: pierwszy indeks + pozycja w layoucie
    const sections: { ym: string | null; start: number }[] = []; // granice miesięcy (oś index)
    let off = 0;
    const push = (row: LayoutRow, h: number) => {
      rowOffsets.push(off);
      layout.push(row);
      off += h;
    };
    const pushCells = (start: number, endExcl: number) => {
      for (let s = start; s < endExcl; s += cols) {
        cellRows.push({ first: s, row: layout.length });
        push({ type: "cells", start: s, end: Math.min(endExcl, s + cols) }, rowHeight);
      }
    };

    if (width > 0 && total > 0) {
      if (sectioned) {
        let acc = 0;
        let lastYear = "";
        for (const [ym, count] of timeline) {
          const year = ym.slice(0, 4);
          if (year !== lastYear) {
            push({ type: "year", year }, HEADER_YEAR);
            lastYear = year;
          }
          push({ type: "month", ym }, HEADER_MONTH);
          sections.push({ ym, start: acc });
          pushCells(acc, acc + count);
          acc += count;
        }
        if (total > acc) {
          push({ type: "month", ym: null }, HEADER_MONTH);
          sections.push({ ym: null, start: acc });
          pushCells(acc, total);
        }
      } else {
        pushCells(0, total);
      }
    }
    return { layout, rowOffsets, totalHeight: off, cellRows, sections };
  }, [sectioned, timeline, total, cols, rowHeight, width]);

  const virtualizer = useVirtualizer({
    count: layout.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (i) => {
      const r = layout[i];
      return r?.type === "year" ? HEADER_YEAR : r?.type === "month" ? HEADER_MONTH : rowHeight;
    },
    overscan: 6,
  });

  // zmiana layoutu (rozmiar kafli, kolumny, sekcje) wymaga przeliczenia pozycji
  useEffect(() => {
    virtualizer.measure();
  }, [layout, virtualizer]);

  const virtualRows = virtualizer.getVirtualItems();

  // prefetch stron dla widocznych wierszy kafli
  useEffect(() => {
    if (!virtualRows.length) return;
    let first = Infinity;
    let last = -1;
    for (const vr of virtualRows) {
      const r = layout[vr.index];
      if (r?.type === "cells") {
        if (r.start < first) first = r.start;
        if (r.end - 1 > last) last = r.end - 1;
      }
    }
    if (last < 0) return;
    for (let p = Math.floor(first / PAGE); p <= Math.floor(last / PAGE); p++) {
      ensurePage(p);
    }
  }, [virtualRows, layout, ensurePage]);

  // geometria dla scrubbera: indeks pliku ↔ offset scrolla (zmienne wysokości)
  const offsetOfIndex = useCallback(
    (fileIdx: number) => {
      if (!cellRows.length) return 0;
      let lo = 0;
      let hi = cellRows.length - 1;
      let ans = 0;
      while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        if (cellRows[mid].first <= fileIdx) {
          ans = mid;
          lo = mid + 1;
        } else {
          hi = mid - 1;
        }
      }
      return rowOffsets[cellRows[ans].row];
    },
    [cellRows, rowOffsets],
  );

  const indexAtOffset = useCallback(
    (scrollTop: number) => {
      if (!layout.length) return 0;
      // największy wiersz layoutu o offsecie ≤ scrollTop
      let lo = 0;
      let hi = layout.length - 1;
      let ans = 0;
      while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        if (rowOffsets[mid] <= scrollTop) {
          ans = mid;
          lo = mid + 1;
        } else {
          hi = mid - 1;
        }
      }
      // przewiń do najbliższego wiersza kafli (pomiń nagłówki)
      for (let i = ans; i < layout.length; i++) {
        const r = layout[i];
        if (r.type === "cells") return r.start;
      }
      return total - 1;
    },
    [layout, rowOffsets, total],
  );

  // trwały wskaźnik rok/miesiąc u góry — aktualizuje się przy przewijaniu
  const [topLabel, setTopLabel] = useState<string | null>(null);
  useEffect(() => {
    if (!sectioned) {
      setTopLabel(null);
      return;
    }
    const el = scrollRef.current;
    if (!el) return;
    const ymOfIndex = (idx: number): string | null => {
      let lo = 0;
      let hi = sections.length - 1;
      let ans = 0;
      while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        if (sections[mid].start <= idx) {
          ans = mid;
          lo = mid + 1;
        } else {
          hi = mid - 1;
        }
      }
      return sections[ans]?.ym ?? null;
    };
    const onScroll = () => {
      const ym = ymOfIndex(indexAtOffset(el.scrollTop));
      setTopLabel(ym === null ? "Bez daty" : monthLabel(ym));
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    onScroll();
    return () => el.removeEventListener("scroll", onScroll);
  }, [sectioned, sections, indexAtOffset]);

  const renderCell = (idx: number) => {
    if (idx >= total) return null;
    const item = pages[Math.floor(idx / PAGE)]?.[idx % PAGE];
    const selected = item ? selection.has(item.id) : false;
    return (
      <div
        key={idx}
        style={{ width: cellSize, height: cellSize }}
        className={`group relative cursor-pointer ${
          selected ? "rounded-[5px] ring-2 ring-accent" : ""
        }`}
        onClick={(e) => {
          if (!item) return;
          if (e.ctrlKey || e.metaKey || e.shiftKey) {
            toggleSelect(item.id, idx, e.shiftKey);
          } else if (item.locked && item.protected_album) {
            useApp.getState().requestUnlock(item.protected_album);
          } else {
            openLightbox(idx);
          }
        }}
        onContextMenu={(e) => {
          if (!item) return;
          e.preventDefault();
          const ids =
            selection.has(item.id) && selection.size > 0 ? [...selection] : [item.id];
          setMenu({ x: e.clientX, y: e.clientY, ids });
        }}
      >
        {item ? (
          <Thumb item={item} />
        ) : (
          <div className="h-full w-full rounded-[3px] bg-surface" />
        )}
        {/* widoczny checkbox: zawsze przy aktywnym zaznaczeniu, inaczej na hover */}
        {item && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              toggleSelect(item.id, idx, false);
            }}
            className={`absolute left-1 top-1 flex h-5 w-5 items-center justify-center rounded-full border text-[11px] transition-opacity ${
              selected
                ? "border-accent bg-accent text-white opacity-100"
                : `border-white/70 bg-black/40 text-transparent hover:text-white ${
                    selection.size > 0 ? "opacity-100" : "opacity-0 group-hover:opacity-100"
                  }`
            }`}
            title="Zaznacz"
          >
            ✓
          </button>
        )}
      </div>
    );
  };

  return (
    <div className="relative h-full">
      <div
        ref={scrollRef}
        className={`h-full overflow-y-auto p-1 ${hasScrubber ? "no-native-scrollbar" : ""}`}
      >
        {total === 0 ? (
          <EmptyState
            icon="🪺"
            title={indexing ? "Indeksowanie biblioteki…" : "Brak plików"}
            description={
              indexing
                ? `Pozostało ${indexing.pending.toLocaleString("pl")} plików.`
                : "Brak plików spełniających bieżące filtry."
            }
          />
        ) : (
          <div className="relative" style={{ height: totalHeight }}>
            {virtualRows.map((vr) => {
              const r = layout[vr.index];
              if (!r) return null;
              if (r.type === "year") {
                return (
                  <div
                    key={vr.key}
                    className="absolute left-0 flex w-full items-end px-1 pb-1 text-lg font-semibold tracking-tight"
                    style={{ top: vr.start, height: vr.size }}
                  >
                    {r.year}
                  </div>
                );
              }
              if (r.type === "month") {
                return (
                  <div
                    key={vr.key}
                    className="absolute left-0 flex w-full items-end px-1 pb-1 text-[13px] font-medium capitalize text-ink-dim"
                    style={{ top: vr.start, height: vr.size }}
                  >
                    {r.ym === null ? "Bez daty" : monthName(r.ym)}
                  </div>
                );
              }
              return (
                <div
                  key={vr.key}
                  className="absolute left-0 flex w-full"
                  style={{ top: vr.start, height: cellSize, gap: GAP }}
                >
                  {Array.from({ length: r.end - r.start }, (_, c) => renderCell(r.start + c))}
                </div>
              );
            })}
          </div>
        )}
      </div>
      {/* trwały wskaźnik rok/miesiąc */}
      {sectioned && total > 0 && topLabel && (
        <div className="pointer-events-none absolute left-2 top-2 z-20 rounded-md bg-raised/90 px-2.5 py-1 text-[12px] font-medium capitalize shadow-lg ring-1 ring-edge backdrop-blur">
          {topLabel}
        </div>
      )}
      {hasScrubber && (
        <DateScrubber
          scrollRef={scrollRef}
          total={total}
          offsetOfIndex={offsetOfIndex}
          indexAtOffset={indexAtOffset}
        />
      )}
      {menu && (
        <div
          className="fixed z-[65] min-w-44 overflow-hidden rounded-md border border-edge bg-raised py-1 text-[13px] shadow-xl"
          style={{ left: menu.x, top: menu.y }}
          onClick={(e) => e.stopPropagation()}
        >
          <button
            onClick={() => {
              setPickerIds(menu.ids);
              setMenu(null);
            }}
            className="block w-full px-3 py-1.5 text-left hover:bg-accent/15"
          >
            Przenieś do folderu…{menu.ids.length > 1 ? ` (${menu.ids.length})` : ""}
          </button>
        </div>
      )}
      {pickerIds && (
        <FolderPicker ids={pickerIds} onClose={() => setPickerIds(null)} />
      )}
    </div>
  );
}
