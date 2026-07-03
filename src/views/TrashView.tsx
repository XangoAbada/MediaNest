import { useEffect, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { Blurhash } from "../components/Blurhash";
import { EmptyState } from "../components/EmptyState";
import { formatSize, useLibrary } from "../stores/library";
import { useApp } from "../stores/app";

interface TrashItem {
  id: number;
  orig_path: string;
  kind: number;
  size: number;
  thumb: string | null;
  blurhash: string | null;
  deleted_at: number;
}

interface Operation {
  id: number;
  kind: string;
  label: string;
  created_at: number;
  undone_at: number | null;
  items: number;
}

const OP_NAMES: Record<string, string> = {
  import: "Import",
  trash: "Przeniesienie do kosza",
  "undo-import": "Cofnięcie importu",
};

function History() {
  const [ops, setOps] = useState<Operation[]>([]);
  const toast = useApp((s) => s.toast);
  const refresh = useLibrary((s) => s.refresh);

  const load = () => invoke<Operation[]>("list_operations").then(setOps);
  useEffect(() => {
    load();
  }, []);

  async function undo(id: number) {
    try {
      const msg = await invoke<string>("undo_operation", { id });
      toast(msg);
      refresh();
    } catch (e) {
      toast(String(e));
    }
    load();
  }

  if (ops.length === 0) {
    return <EmptyState icon="🕓" title="Brak operacji" />;
  }
  return (
    <div className="mx-auto max-w-2xl space-y-2 p-4">
      {ops.map((op) => (
        <div
          key={op.id}
          className="flex items-center gap-3 rounded-lg border border-edge bg-surface px-4 py-2.5 text-[13px]"
        >
          <div className="min-w-0 flex-1">
            <span className="font-medium">{OP_NAMES[op.kind] ?? op.kind}</span>
            <span className="ml-2 text-ink-dim">
              {op.items} plików · {op.label}
            </span>
            <div className="text-[11px] text-ink-faint">
              {new Date(op.created_at * 1000).toLocaleString("pl")}
              {op.undone_at && " · cofnięta"}
            </div>
          </div>
          {!op.undone_at && (op.kind === "import" || op.kind === "trash") && (
            <button
              onClick={() => undo(op.id)}
              className="shrink-0 rounded-md border border-edge px-3 py-1 hover:border-ink-faint"
            >
              Cofnij
            </button>
          )}
        </div>
      ))}
    </div>
  );
}

export function TrashView() {
  const [tab, setTab] = useState<"trash" | "history">("trash");
  const [items, setItems] = useState<TrashItem[]>([]);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const toast = useApp((s) => s.toast);
  const refresh = useLibrary((s) => s.refresh);

  const load = () =>
    invoke<TrashItem[]>("list_trash").then((rows) => {
      setItems(rows);
      setSelected(new Set());
    });
  useEffect(() => {
    load();
  }, [tab]);

  const ids = selected.size > 0 ? [...selected] : items.map((i) => i.id);

  async function restore() {
    const n = await invoke<number>("restore_trash", { ids });
    toast(`Przywrócono ${n} plików`);
    refresh();
    load();
  }

  async function empty() {
    if (!window.confirm(`Trwale usunąć ${ids.length} plików? Tego nie można cofnąć.`)) return;
    const n = await invoke<number>("empty_trash", {
      ids: selected.size > 0 ? [...selected] : null,
    });
    toast(`Usunięto trwale ${n} plików`);
    load();
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-1 border-b border-edge bg-surface px-4 py-2">
        {(["trash", "history"] as const).map((t) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className={`rounded-md px-3 py-1 text-[13px] ${
              tab === t ? "bg-accent/15 text-ink" : "text-ink-dim hover:text-ink"
            }`}
          >
            {t === "trash" ? `Kosz (${items.length})` : "Historia operacji"}
          </button>
        ))}
        {tab === "trash" && items.length > 0 && (
          <div className="ml-auto flex gap-2 text-[12px]">
            <button
              onClick={restore}
              className="rounded-md border border-edge px-3 py-1 hover:border-ink-faint"
            >
              Przywróć {selected.size > 0 ? `(${selected.size})` : "wszystko"}
            </button>
            <button
              onClick={empty}
              className="rounded-md border border-danger/40 px-3 py-1 text-danger hover:bg-danger/10"
            >
              Usuń trwale {selected.size > 0 ? `(${selected.size})` : "wszystko"}
            </button>
          </div>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {tab === "history" ? (
          <History />
        ) : items.length === 0 ? (
          <EmptyState
            icon="🗑"
            title="Kosz jest pusty"
            description="Usunięte pliki trafiają tutaj i można je przywrócić."
          />
        ) : (
          <div className="flex flex-wrap gap-2 p-3">
            {items.map((item) => {
              const sel = selected.has(item.id);
              return (
                <div
                  key={item.id}
                  onClick={() => {
                    const next = new Set(selected);
                    if (sel) next.delete(item.id);
                    else next.add(item.id);
                    setSelected(next);
                  }}
                  title={`${item.orig_path} (${formatSize(item.size)})`}
                  className={`relative h-32 w-32 cursor-pointer overflow-hidden rounded-md bg-surface ${
                    sel ? "ring-2 ring-accent" : ""
                  }`}
                >
                  {item.blurhash && (
                    <Blurhash hash={item.blurhash} className="absolute inset-0 h-full w-full" />
                  )}
                  {item.thumb && (
                    <img
                      src={convertFileSrc(item.thumb, "thumb")}
                      loading="lazy"
                      className="absolute inset-0 h-full w-full object-cover"
                    />
                  )}
                  {!item.thumb && !item.blurhash && (
                    <div className="absolute inset-0 flex items-center justify-center text-2xl opacity-30">
                      {item.kind === 1 ? "🎬" : "🖼"}
                    </div>
                  )}
                  {sel && (
                    <span className="absolute right-1 top-1 flex h-5 w-5 items-center justify-center rounded-full bg-accent text-[11px] text-white">
                      ✓
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
