import { useEffect, useRef } from "react";
import { decode } from "blurhash";

const SIZE = 32;

export function Blurhash({ hash, className }: { hash: string; className?: string }) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    try {
      const pixels = decode(hash, SIZE, SIZE);
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      const imageData = ctx.createImageData(SIZE, SIZE);
      imageData.data.set(pixels);
      ctx.putImageData(imageData, 0, 0);
    } catch {
      // niepoprawny hash — zostaje puste tło
    }
  }, [hash]);

  return <canvas ref={ref} width={SIZE} height={SIZE} className={className} />;
}
