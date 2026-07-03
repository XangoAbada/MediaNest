import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  useLibrary,
  formatDuration,
  formatSize,
  type FileInfo,
} from "../stores/library";
import { Thumb } from "./Thumb";
import { FolderPicker } from "./FolderPicker";
import { PAGE } from "../stores/library";

const SLIDESHOW_MS = 4000;

function TagEditor({ id }: { id: number }) {
  const [tags, setTags] = useState<[number, string][]>([]);
  const [input, setInput] = useState("");

  const load = () => invoke<[number, string][]>("get_file_tags", { id }).then(setTags);
  useEffect(() => {
    setTags([]);
    load();
  }, [id]);

  return (
    <div className="mb-3">
      <div className="text-[11px] uppercase tracking-wider text-ink-faint">Tagi</div>
      <div className="mt-1 flex flex-wrap gap-1">
        {tags.map(([tagId, name]) => (
          <span
            key={tagId}
            className="flex items-center gap-1 rounded-full bg-raised px-2 py-0.5 text-[11px] text-ink-dim"
          >
            #{name}
            <button
              onClick={() => invoke("untag_file", { id, tagId }).then(load)}
              className="text-ink-faint hover:text-danger"
            >
              ✕
            </button>
          </span>
        ))}
      </div>
      <input
        value={input}
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation(); // nie wyzwalaj skrótów lightboxa
          if (e.key === "Enter" && input.trim()) {
            invoke("tag_file", { id, name: input.trim() }).then(() => {
              setInput("");
              load();
            });
          }
        }}
        placeholder="Dodaj tag (Enter)…"
        className="mt-1.5 w-full rounded-md border border-edge bg-app px-2 py-1 text-[12px] outline-none placeholder:text-ink-faint focus:border-accent"
      />
    </div>
  );
}

function InfoPanel({ id }: { id: number }) {
  const [info, setInfo] = useState<FileInfo | null>(null);

  useEffect(() => {
    setInfo(null);
    invoke<FileInfo>("get_file_info", { id }).then(setInfo);
  }, [id]);

  if (!info) return null;
  const rows: [string, string][] = [
    ["Nazwa", info.name],
    ["Ścieżka", info.path],
    ["Rozmiar", formatSize(info.size)],
    ...(info.width && info.height
      ? ([["Wymiary", `${info.width} × ${info.height}`]] as [string, string][])
      : []),
    ...(info.duration
      ? ([["Czas trwania", formatDuration(info.duration)]] as [string, string][])
      : []),
    ...(info.taken_at
      ? ([
          [
            "Data",
            new Date(info.taken_at * 1000).toLocaleString("pl", {
              dateStyle: "long",
              timeStyle: "short",
            }),
          ],
        ] as [string, string][])
      : []),
  ];
  return (
    <div className="w-72 shrink-0 overflow-y-auto border-l border-edge bg-surface p-4">
      <h3 className="mb-3 text-sm font-semibold">Informacje</h3>
      {rows.map(([label, value]) => (
        <div key={label} className="mb-3">
          <div className="text-[11px] uppercase tracking-wider text-ink-faint">{label}</div>
          <div className="break-all font-mono text-[12px] text-ink-dim">{value}</div>
        </div>
      ))}
      <TagEditor id={id} />
    </div>
  );
}

export function Lightbox() {
  const { lightbox, total, itemAt, ensurePage, setLightbox, closeLightbox } = useLibrary();
  const index = lightbox as number;
  const item = itemAt(index);

  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [showInfo, setShowInfo] = useState(false);
  const [moving, setMoving] = useState(false);
  const [slideshow, setSlideshow] = useState(false);
  const [uiVisible, setUiVisible] = useState(true);
  // wideo: URL gotowy do odtwarzania (po ew. transkodowaniu) + postęp (0–1, -1 = błąd)
  const [videoSrc, setVideoSrc] = useState<string | null>(null);
  const [videoProgress, setVideoProgress] = useState<number | null>(null);
  const uiTimer = useRef<number>(0);
  const dragRef = useRef<{ x: number; y: number } | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  // strony: bieżąca + sąsiednie
  useEffect(() => {
    ensurePage(Math.floor(index / PAGE));
    ensurePage(Math.floor(Math.max(0, index - 3) / PAGE));
    ensurePage(Math.floor(Math.min(total - 1, index + 3) / PAGE));
  }, [index, total, ensurePage]);

  // reset zoom przy zmianie pliku
  useEffect(() => {
    setZoom(1);
    setPan({ x: 0, y: 0 });
  }, [index]);

  // przygotowanie wideo: backend decyduje czy serwować oryginał, czy transkodować
  useEffect(() => {
    if (!item || item.kind !== 1) {
      setVideoSrc(null);
      setVideoProgress(null);
      return;
    }
    const vid = item.id;
    setVideoSrc(null);
    setVideoProgress(null);
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    listen<[number, number]>("transcode-progress", (e) => {
      if (e.payload[0] === vid) setVideoProgress(e.payload[1]);
    }).then((u) => (cancelled ? u() : (unlisten = u)));
    invoke<string>("prepare_video", { id: vid })
      .then((ret) => !cancelled && setVideoSrc(convertFileSrc(ret, "media")))
      .catch(() => !cancelled && setVideoProgress(-1));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [item?.id, item?.kind]);

  // preload pełnych zdjęć sąsiadów — strzałki działają bez czekania
  useEffect(() => {
    for (const i of [index + 1, index - 1]) {
      const n = itemAt(i);
      if (n && n.kind === 0 && !n.locked) {
        const img = new Image();
        img.src = convertFileSrc(String(n.id), "media");
      }
    }
  }, [index, itemAt]);

  const prev = useCallback(() => setLightbox(index - 1), [index, setLightbox]);
  const next = useCallback(() => setLightbox(index + 1), [index, setLightbox]);

  // pokaz slajdów
  useEffect(() => {
    if (!slideshow) return;
    const t = window.setInterval(() => {
      const s = useLibrary.getState();
      s.setLightbox(s.lightbox === s.total - 1 ? 0 : (s.lightbox as number) + 1);
    }, SLIDESHOW_MS);
    return () => window.clearInterval(t);
  }, [slideshow]);

  // chowanie UI po bezczynności myszy
  const pokeUi = useCallback(() => {
    setUiVisible(true);
    window.clearTimeout(uiTimer.current);
    uiTimer.current = window.setTimeout(() => setUiVisible(false), 2000);
  }, []);

  // pełny ekran (przycisk, klawisz F, dwuklik) + synchronizacja ikony
  const [isFs, setIsFs] = useState(false);
  const toggleFs = useCallback(() => {
    if (document.fullscreenElement) document.exitFullscreen();
    else rootRef.current?.requestFullscreen();
  }, []);
  useEffect(() => {
    const onFs = () => setIsFs(!!document.fullscreenElement);
    document.addEventListener("fullscreenchange", onFs);
    return () => document.removeEventListener("fullscreenchange", onFs);
  }, []);

  // skróty klawiszowe
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (moving) return; // picker folderu przejmuje klawiaturę
      switch (e.key) {
        case "m":
        case "M":
          setMoving(true);
          break;
        case "Escape":
          if (document.fullscreenElement) document.exitFullscreen();
          else closeLightbox();
          break;
        case "ArrowLeft":
          prev();
          break;
        case "ArrowRight":
          next();
          break;
        case "i":
        case "I":
          setShowInfo((v) => !v);
          break;
        case "f":
        case "F":
          toggleFs();
          break;
        case "s":
        case "S":
          setSlideshow((v) => !v);
          break;
        case "0":
        case "1":
        case "2":
        case "3":
        case "4":
        case "5": {
          const s = useLibrary.getState();
          const current = s.itemAt(s.lightbox as number);
          if (current) {
            invoke("set_rating", { id: current.id, rating: Number(e.key) });
          }
          break;
        }
        case "Delete": {
          const s = useLibrary.getState();
          const current = s.itemAt(s.lightbox as number);
          if (current) {
            invoke("trash_files", { ids: [current.id] }).then(() => {
              if ((s.lightbox as number) >= s.total - 1) s.closeLightbox();
              s.refresh();
            });
          }
          break;
        }
        default:
          return;
      }
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [prev, next, closeLightbox, moving, toggleFs]);

  if (!item) {
    return <div className="fixed inset-0 z-50 bg-black" onClick={closeLightbox} />;
  }

  const src = convertFileSrc(String(item.id), "media");
  const isVideo = item.kind === 1;

  return (
    <div
      ref={rootRef}
      className="fixed inset-0 z-50 flex bg-black"
      onMouseMove={pokeUi}
    >
      <div className="relative flex min-w-0 flex-1 flex-col">
        {/* Obszar mediów */}
        <div
          className="relative min-h-0 flex-1 overflow-hidden"
          onWheel={(e) => {
            if (isVideo) return;
            const factor = e.deltaY < 0 ? 1.2 : 1 / 1.2;
            setZoom((z) => Math.min(8, Math.max(1, z * factor)));
            if (zoom * factor <= 1) setPan({ x: 0, y: 0 });
          }}
          onPointerDown={(e) => {
            if (zoom > 1) {
              dragRef.current = { x: e.clientX - pan.x, y: e.clientY - pan.y };
            }
          }}
          onPointerMove={(e) => {
            if (dragRef.current) {
              setPan({ x: e.clientX - dragRef.current.x, y: e.clientY - dragRef.current.y });
            }
          }}
          onPointerUp={() => (dragRef.current = null)}
          onDoubleClick={() => {
            if (zoom > 1) {
              setZoom(1);
              setPan({ x: 0, y: 0 });
            } else {
              toggleFs();
            }
          }}
        >
          {isVideo ? (
            videoSrc ? (
              <video
                key={item.id}
                src={videoSrc}
                controls
                autoPlay
                className="absolute inset-0 h-full w-full"
              />
            ) : (
              <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 text-white/80">
                <div className="text-sm">
                  {videoProgress === -1
                    ? "Nie udało się przygotować wideo"
                    : videoProgress != null
                      ? `Przygotowywanie wideo… ${Math.round(videoProgress * 100)}%`
                      : "Przygotowywanie wideo…"}
                </div>
                {videoProgress != null && videoProgress >= 0 && (
                  <div className="h-1 w-48 overflow-hidden rounded-full bg-white/20">
                    <div
                      className="h-full bg-accent transition-[width] duration-200"
                      style={{ width: `${Math.round(videoProgress * 100)}%` }}
                    />
                  </div>
                )}
              </div>
            )
          ) : (
            // bez key: przy zmianie src przeglądarka trzyma poprzednią bitmapę
            // do czasu zdekodowania nowej — zero czarnego mrugnięcia
            <img
              src={src}
              draggable={false}
              className="absolute inset-0 h-full w-full object-contain"
              style={{
                transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
                cursor: zoom > 1 ? "grab" : "default",
                transition: dragRef.current ? "none" : "transform 120ms ease-out",
              }}
            />
          )}

          {/* Nakładka UI */}
          <div
            className={`pointer-events-none absolute inset-0 transition-opacity duration-300 ${
              uiVisible ? "opacity-100" : "opacity-0"
            }`}
          >
            <div className="pointer-events-auto absolute left-0 right-0 top-0 flex items-center gap-3 bg-gradient-to-b from-black/70 to-transparent px-4 py-3">
              <button onClick={closeLightbox} className="text-lg text-white/80 hover:text-white">
                ✕
              </button>
              <span className="truncate text-sm text-white/90">{item.name}</span>
              <span className="text-[12px] text-white/50">
                {index + 1} / {total.toLocaleString("pl")}
              </span>
              <div className="ml-auto flex items-center gap-3 text-[13px]">
                <button
                  onClick={() => setSlideshow((v) => !v)}
                  className={`hover:text-white ${slideshow ? "text-accent-hover" : "text-white/70"}`}
                  title="Pokaz slajdów (S)"
                >
                  {slideshow ? "⏸ Pokaz" : "▶ Pokaz"}
                </button>
                <button
                  onClick={() => setMoving(true)}
                  className="text-white/70 hover:text-white"
                  title="Przenieś do folderu (M)"
                >
                  🗀 Przenieś
                </button>
                <button
                  onClick={toggleFs}
                  className="text-white/70 hover:text-white"
                  title="Pełny ekran (F)"
                >
                  {isFs ? "🡼 Zamknij" : "⛶ Pełny ekran"}
                </button>
                <button
                  onClick={() => setShowInfo((v) => !v)}
                  className={`hover:text-white ${showInfo ? "text-accent-hover" : "text-white/70"}`}
                  title="Informacje (I)"
                >
                  ⓘ Info
                </button>
              </div>
            </div>
            {index > 0 && (
              <button
                onClick={prev}
                className="pointer-events-auto absolute left-2 top-1/2 -translate-y-1/2 rounded-full bg-black/50 px-3 py-2 text-xl text-white/80 hover:bg-black/80 hover:text-white"
              >
                ‹
              </button>
            )}
            {index < total - 1 && (
              <button
                onClick={next}
                className="pointer-events-auto absolute right-2 top-1/2 -translate-y-1/2 rounded-full bg-black/50 px-3 py-2 text-xl text-white/80 hover:bg-black/80 hover:text-white"
              >
                ›
              </button>
            )}
          </div>
        </div>

        {/* Filmstrip */}
        <div
          className={`flex shrink-0 items-center justify-center gap-1 overflow-hidden bg-black/90 py-2 transition-opacity duration-300 ${
            uiVisible ? "opacity-100" : "opacity-0"
          }`}
        >
          {Array.from({ length: 11 }, (_, i) => index - 5 + i)
            .filter((i) => i >= 0 && i < total)
            .map((i) => {
              const n = itemAt(i);
              return (
                <div
                  key={i}
                  onClick={() => setLightbox(i)}
                  className={`h-12 w-12 shrink-0 cursor-pointer overflow-hidden rounded-[3px] ${
                    i === index ? "ring-2 ring-accent" : "opacity-60 hover:opacity-100"
                  }`}
                >
                  {n && <Thumb item={n} scrub={false} />}
                </div>
              );
            })}
        </div>
      </div>

      {showInfo && <InfoPanel id={item.id} />}
      {moving && (
        <FolderPicker
          ids={[item.id]}
          onClose={() => setMoving(false)}
          onMoved={() => {
            const s = useLibrary.getState();
            if ((s.lightbox as number) >= s.total - 1) s.closeLightbox();
          }}
        />
      )}
    </div>
  );
}
