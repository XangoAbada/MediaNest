import { useEffect, useMemo, useRef, useState } from "react";
import { useLibrary } from "../stores/library";

const MONTHS = [
  "sty", "lut", "mar", "kwi", "maj", "cze",
  "lip", "sie", "wrz", "paź", "lis", "gru",
];

function monthYear(ym: string): string {
  const [y, m] = ym.split("-");
  return `${MONTHS[Number(m) - 1]} ${y}`;
}

/**
 * Pionowy scrubber daty (jak w aplikacji Zdjęcia): przeciąganie przenosi
 * siatkę do wybranej daty, dymek pokazuje bieżący miesiąc. Cała geometria
 * pracuje na osi indeks-pliku/total, więc znaczniki lat i uchwyt są zgodne.
 */
export function DateScrubber({
  scrollRef,
  total,
  offsetOfIndex,
  indexAtOffset,
}: {
  scrollRef: React.RefObject<HTMLDivElement | null>;
  total: number;
  offsetOfIndex: (fileIdx: number) => number; // indeks pliku → scrollTop
  indexAtOffset: (scrollTop: number) => number; // scrollTop → indeks pliku
}) {
  const timeline = useLibrary((s) => s.timeline);
  const trackRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);
  const scrollEndTimer = useRef(0);

  const [thumbFrac, setThumbFrac] = useState(0);
  const [topDate, setTopDate] = useState<string | null>(null);
  const [hoverFrac, setHoverFrac] = useState<number | null>(null);
  const [active, setActive] = useState(false);
  const [scrolling, setScrolling] = useState(false);

  // suma narastająca miesięcy + pozycje znaczników lat (oś index/total)
  const { starts, datedTotal, years } = useMemo(() => {
    let acc = 0;
    const starts: number[] = [];
    const years: { year: string; frac: number }[] = [];
    let lastYear = "";
    timeline.forEach(([ym], i) => {
      starts.push(acc);
      acc += timeline[i][1];
      const y = ym.slice(0, 4);
      if (y !== lastYear) {
        years.push({ year: y, frac: total ? starts[i] / total : 0 });
        lastYear = y;
      }
    });
    return { starts, datedTotal: acc, years };
  }, [timeline, total]);

  function dateOfIndex(idx: number): string | null {
    if (idx >= datedTotal) return null; // pliki bez daty na końcu
    let lo = 0;
    let hi = starts.length - 1;
    let ans = 0;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (starts[mid] <= idx) {
        ans = mid;
        lo = mid + 1;
      } else {
        hi = mid - 1;
      }
    }
    return timeline[ans]?.[0] ?? null;
  }

  // uchwyt i data podążają za scrollem siatki (kółko, klawiatura, drag)
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onScroll = () => {
      const topIdx = Math.min(total - 1, indexAtOffset(el.scrollTop));
      setThumbFrac(total ? topIdx / total : 0);
      setTopDate(dateOfIndex(topIdx));
      setScrolling(true);
      window.clearTimeout(scrollEndTimer.current);
      scrollEndTimer.current = window.setTimeout(() => setScrolling(false), 700);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    onScroll();
    return () => el.removeEventListener("scroll", onScroll);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scrollRef, total, indexAtOffset, starts, datedTotal]);

  if (timeline.length === 0 || total < 50) return null;

  const fracFromEvent = (e: React.PointerEvent): number => {
    const rect = trackRef.current!.getBoundingClientRect();
    return Math.min(1, Math.max(0, (e.clientY - rect.top) / rect.height));
  };
  const scrollToFrac = (f: number) => {
    const el = scrollRef.current;
    if (!el) return;
    const idx = Math.min(total - 1, Math.round(f * total));
    el.scrollTop = offsetOfIndex(idx);
  };

  const bubbleFrac = hoverFrac ?? thumbFrac;
  const bubbleDate =
    hoverFrac != null
      ? dateOfIndex(Math.min(total - 1, Math.round(hoverFrac * total)))
      : topDate;
  const showBubble = (active || scrolling) && bubbleDate;

  return (
    // outer: przezroczysty dla zdarzeń, żeby nie blokować klikania miniaturek —
    // tylko pasek (poniżej) przechwytuje wskaźnik
    <div className="pointer-events-none absolute right-0 top-0 z-20 h-full w-11 select-none">
      {/* dymek z datą — po lewej stronie paska */}
      {showBubble && (
        <div
          className="absolute right-11 z-30 -translate-y-1/2 whitespace-nowrap rounded-md bg-raised px-2.5 py-1 text-[12px] font-medium capitalize shadow-lg ring-1 ring-edge"
          style={{ top: `${bubbleFrac * 100}%` }}
        >
          {monthYear(bubbleDate!)}
        </div>
      )}

      {/* pasek + znaczniki lat + uchwyt */}
      <div
        ref={trackRef}
        className="pointer-events-auto absolute inset-y-2 right-1.5 w-6 cursor-pointer"
        onPointerEnter={() => setActive(true)}
        onPointerLeave={() => {
          if (!draggingRef.current) {
            setActive(false);
            setHoverFrac(null);
          }
        }}
        onPointerDown={(e) => {
          draggingRef.current = true;
          e.currentTarget.setPointerCapture(e.pointerId);
          const f = fracFromEvent(e);
          setHoverFrac(f);
          scrollToFrac(f);
        }}
        onPointerMove={(e) => {
          const f = fracFromEvent(e);
          setHoverFrac(f);
          if (draggingRef.current) scrollToFrac(f);
        }}
        onPointerUp={(e) => {
          draggingRef.current = false;
          e.currentTarget.releasePointerCapture(e.pointerId);
        }}
      >
        {/* etykiety lat — pojawiają się na hover/drag */}
        <div
          className={`absolute inset-0 transition-opacity duration-150 ${
            active ? "opacity-100" : "opacity-0"
          }`}
        >
          {years.map(({ year, frac }) => (
            <div
              key={year}
              className="absolute right-7 -translate-y-1/2 whitespace-nowrap text-[10px] font-medium text-ink-faint"
              style={{ top: `${frac * 100}%` }}
            >
              {year}
            </div>
          ))}
          {years.map(({ year, frac }) => (
            <div
              key={`tick-${year}`}
              className="absolute right-0 h-px w-2 -translate-y-1/2 bg-ink-faint/50"
              style={{ top: `${frac * 100}%` }}
            />
          ))}
        </div>

        {/* uchwyt */}
        <div
          className={`absolute right-0 h-8 w-1.5 -translate-y-1/2 rounded-full transition-colors ${
            active || scrolling ? "bg-accent" : "bg-ink-faint/40"
          }`}
          style={{ top: `${thumbFrac * 100}%` }}
        />
      </div>
    </div>
  );
}
