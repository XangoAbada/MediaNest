import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useLibrary, PAGE } from "../stores/library";
import { useApp } from "../stores/app";
import { Thumb } from "./Thumb";
import { EmptyState } from "./EmptyState";
import { DateScrubber } from "./DateScrubber";

const GAP = 4;

export function Grid() {
  const { total, pages, ensurePage, cellSize, indexing, openLightbox, selection, toggleSelect } =
    useLibrary();
  const sort = useLibrary((s) => s.q.sort);
  const timeline = useLibrary((s) => s.timeline);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);

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
  // rezerwa na scrubber po prawej, żeby ostatnia kolumna go nie chowała
  const SCRUB = 36;
  const usable = width - (hasScrubber ? SCRUB : 0);
  const cols = Math.max(1, Math.floor((usable - GAP) / (cellSize + GAP)));
  const rowCount = width > 0 ? Math.ceil(total / cols) : 0;
  const rowHeight = cellSize + GAP;

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 4,
  });

  // zmiana rozmiaru kafli wymaga przeliczenia pozycji wierszy
  useEffect(() => {
    virtualizer.measure();
  }, [rowHeight, cols, virtualizer]);

  const virtualRows = virtualizer.getVirtualItems();

  useEffect(() => {
    if (!virtualRows.length || !cols) return;
    const first = virtualRows[0].index * cols;
    const last = Math.min(total - 1, (virtualRows[virtualRows.length - 1].index + 1) * cols - 1);
    for (let p = Math.floor(first / PAGE); p <= Math.floor(last / PAGE); p++) {
      ensurePage(p);
    }
  }, [virtualRows, cols, total, ensurePage]);

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
        <div className="relative" style={{ height: virtualizer.getTotalSize() }}>
          {virtualRows.map((row) => (
            <div
              key={row.key}
              className="absolute left-0 flex w-full"
              style={{ top: row.start, height: cellSize, gap: GAP }}
            >
              {Array.from({ length: cols }, (_, c) => {
                const idx = row.index * cols + c;
                if (idx >= total) return null;
                const item = pages[Math.floor(idx / PAGE)]?.[idx % PAGE];
                const selected = item ? selection.has(item.id) : false;
                return (
                  <div
                    key={c}
                    style={{ width: cellSize, height: cellSize }}
                    className={`relative cursor-pointer ${
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
                  >
                    {item ? (
                      <Thumb item={item} />
                    ) : (
                      <div className="h-full w-full rounded-[3px] bg-surface" />
                    )}
                    {selected && (
                      <span className="absolute right-1 top-1 flex h-5 w-5 items-center justify-center rounded-full bg-accent text-[11px] text-white">
                        ✓
                      </span>
                    )}
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      )}
      </div>
      {hasScrubber && (
        <DateScrubber
          scrollRef={scrollRef}
          total={total}
          cols={cols}
          rowHeight={rowHeight}
        />
      )}
    </div>
  );
}
