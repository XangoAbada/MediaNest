import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { EmptyState } from "../components/EmptyState";
import { Grid } from "../components/Grid";
import { useLibrary } from "../stores/library";

export function TagsView() {
  const [tags, setTags] = useState<[number, string, number][] | null>(null);
  const [open, setOpen] = useState<[number, string] | null>(null);
  const { resetQuery } = useLibrary();

  useEffect(() => {
    invoke<[number, string, number][]>("list_tags").then(setTags);
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
            ← Tagi
          </button>
          <span className="font-medium">#{open[1]}</span>
        </div>
        <div className="min-h-0 flex-1">
          <Grid />
        </div>
      </div>
    );
  }

  if (tags && tags.length === 0) {
    return (
      <EmptyState
        icon="🏷"
        title="Brak tagów"
        description="Tagi dodasz w panelu informacji (klawisz I) w pełnym widoku zdjęcia."
      />
    );
  }

  return (
    <div className="mx-auto max-w-3xl p-6">
      <h2 className="mb-4 text-lg font-semibold tracking-tight">Tagi</h2>
      <div className="flex flex-wrap gap-2">
        {tags?.map(([id, name, count]) => (
          <button
            key={id}
            onClick={() => {
              setOpen([id, name]);
              resetQuery({ parent: null, tag_id: id });
            }}
            className="rounded-full border border-edge bg-surface px-3 py-1.5 text-[13px] text-ink-dim transition-colors hover:border-accent hover:text-ink"
          >
            #{name} <span className="text-ink-faint">{count}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
