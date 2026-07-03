import { useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Blurhash } from "./Blurhash";
import { formatDuration, type FileItem } from "../stores/library";

const SPRITE_FRAMES = 10;

export function Thumb({ item, scrub = true }: { item: FileItem; scrub?: boolean }) {
  const [loaded, setLoaded] = useState(false);
  const [spriteOk, setSpriteOk] = useState(false);
  const [frame, setFrame] = useState(0);
  const [hover, setHover] = useState(false);
  const spriteTried = useRef(false);

  const canScrub = scrub && item.kind === 1 && item.thumb && !item.locked;

  // zablokowany plik chroniony: tylko blurhash + kłódka, bez żądania miniaturki
  if (item.locked) {
    return (
      <div className="relative h-full w-full overflow-hidden rounded-[3px] bg-surface">
        {item.blurhash && (
          <Blurhash hash={item.blurhash} className="absolute inset-0 h-full w-full" />
        )}
        <div className="absolute inset-0 flex items-center justify-center">
          <span className="rounded-full bg-black/50 p-2 text-lg">🔒</span>
        </div>
      </div>
    );
  }

  function onEnter() {
    if (!canScrub) return;
    setHover(true);
    if (!spriteTried.current) {
      spriteTried.current = true;
      const img = new Image();
      img.onload = () => setSpriteOk(true);
      img.src = convertFileSrc(`${item.thumb}.sprite`, "thumb");
    }
  }

  function onMove(e: React.MouseEvent<HTMLDivElement>) {
    if (!canScrub || !spriteOk) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const t = (e.clientX - rect.left) / rect.width;
    setFrame(Math.min(SPRITE_FRAMES - 1, Math.max(0, Math.floor(t * SPRITE_FRAMES))));
  }

  const scrubbing = hover && spriteOk;

  return (
    <div
      className="relative h-full w-full overflow-hidden rounded-[3px] bg-surface"
      title={item.name}
      onMouseEnter={onEnter}
      onMouseLeave={() => setHover(false)}
      onMouseMove={onMove}
    >
      {item.blurhash && !loaded && (
        <Blurhash hash={item.blurhash} className="absolute inset-0 h-full w-full" />
      )}
      {item.thumb ? (
        <img
          src={convertFileSrc(item.thumb, "thumb")}
          loading="lazy"
          decoding="async"
          onLoad={() => setLoaded(true)}
          className={`absolute inset-0 h-full w-full object-cover transition-opacity duration-150 ${
            loaded && !scrubbing ? "opacity-100" : "opacity-0"
          }`}
        />
      ) : (
        <div className="absolute inset-0 flex items-center justify-center text-2xl opacity-30">
          {item.kind === 1 ? "🎬" : "🖼"}
        </div>
      )}
      {scrubbing && (
        <div
          className="absolute inset-0"
          style={{
            backgroundImage: `url(${convertFileSrc(`${item.thumb}.sprite`, "thumb")})`,
            backgroundSize: `${SPRITE_FRAMES * 100}% 100%`,
            backgroundPosition: `${(frame * 100) / (SPRITE_FRAMES - 1)}% center`,
          }}
        />
      )}
      {item.kind === 1 && item.duration != null && (
        <span className="absolute bottom-1 right-1 rounded bg-black/60 px-1 text-[10px] leading-4 text-white">
          {formatDuration(item.duration)}
        </span>
      )}
    </div>
  );
}
