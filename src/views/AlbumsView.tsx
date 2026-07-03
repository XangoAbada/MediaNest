import { useEffect, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { EmptyState } from "../components/EmptyState";
import { Grid } from "../components/Grid";
import { useLibrary } from "../stores/library";
import { useApp } from "../stores/app";

interface Album {
  id: number;
  name: string;
  type: string;
  folder_path: string | null;
  count: number;
  cover: string | null;
}

function CreateForm({ onCreated }: { onCreated: () => void }) {
  const [name, setName] = useState("");
  const [type, setType] = useState("virtual");
  const [password, setPassword] = useState("");
  const [password2, setPassword2] = useState("");
  const toast = useApp((s) => s.toast);

  async function create() {
    try {
      if (type === "protected") {
        if (password !== password2) {
          toast("Hasła się różnią");
          return;
        }
        await invoke("create_protected_album", { name, password });
      } else {
        await invoke("create_album", { name, albumType: type });
      }
      onCreated();
    } catch (e) {
      toast(String(e));
    }
  }

  return (
    <div className="mb-4 rounded-lg border border-edge bg-surface p-4">
      <div className="flex items-end gap-3">
        <label className="flex-1">
          <span className="text-[12px] text-ink-dim">Nazwa</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
            className="mt-1 w-full rounded-md border border-edge bg-app px-2.5 py-1.5 text-[13px] outline-none focus:border-accent"
          />
        </label>
        <label>
          <span className="text-[12px] text-ink-dim">Typ</span>
          <select
            value={type}
            onChange={(e) => setType(e.target.value)}
            className="mt-1 block rounded-md border border-edge bg-raised px-2 py-1.5 text-[13px] outline-none"
          >
            <option value="virtual">Wirtualny (referencje)</option>
            <option value="folder">Folderowy (przenosi pliki)</option>
            <option value="protected">Chroniony hasłem (szyfruje pliki)</option>
          </select>
        </label>
        <button
          onClick={create}
          disabled={!name.trim() || (type === "protected" && password.length < 4)}
          className="rounded-md bg-accent px-4 py-1.5 text-[13px] text-white hover:bg-accent-hover disabled:opacity-50"
        >
          Utwórz
        </button>
      </div>
      {type === "protected" && (
        <div className="mt-3">
          <div className="flex gap-3">
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="Hasło (min. 4 znaki)"
              className="flex-1 rounded-md border border-edge bg-app px-2.5 py-1.5 text-[13px] outline-none focus:border-accent"
            />
            <input
              type="password"
              value={password2}
              onChange={(e) => setPassword2(e.target.value)}
              placeholder="Powtórz hasło"
              className="flex-1 rounded-md border border-edge bg-app px-2.5 py-1.5 text-[13px] outline-none focus:border-accent"
            />
          </div>
          <p className="mt-2 text-[12px] text-warning">
            ⚠ Pliki w albumie zostaną zaszyfrowane na dysku. Utrata hasła oznacza
            bezpowrotną utratę tych plików — nie ma żadnej metody odzyskania.
          </p>
        </div>
      )}
    </div>
  );
}

function ProtectedAlbumBar({
  album,
  unlocked,
  onBack,
  onChanged,
}: {
  album: Album;
  unlocked: boolean;
  onBack: () => void;
  onChanged: () => void;
}) {
  const { selection, clearSelection, refresh } = useLibrary();
  const { toast, requestUnlock } = useApp();
  const [changing, setChanging] = useState(false);
  const [newPass, setNewPass] = useState("");

  return (
    <div className="border-b border-edge bg-surface px-4 py-2">
      <div className="flex items-center gap-3 text-[13px]">
        <button onClick={onBack} className="text-ink-dim hover:text-ink">
          ← Albumy
        </button>
        <span className="font-medium">🔒 {album.name}</span>
        {unlocked ? (
          <>
            <span className="rounded bg-success/15 px-2 py-0.5 text-[11px] text-success">
              odblokowany
            </span>
            {selection.size > 0 && (
              <button
                onClick={async () => {
                  const n = await invoke<number>("unprotect_files", {
                    albumId: album.id,
                    fileIds: [...selection],
                  });
                  toast(`Odszyfrowano ${n} plików`);
                  clearSelection();
                  refresh();
                }}
                className="rounded-md border border-edge px-3 py-1 text-[12px] hover:border-ink-faint"
              >
                Wyjmij zaznaczone ({selection.size})
              </button>
            )}
            <div className="ml-auto flex items-center gap-2">
              <button
                onClick={() => setChanging(!changing)}
                className="rounded-md border border-edge px-3 py-1 text-[12px] hover:border-ink-faint"
              >
                Zmień hasło
              </button>
              <button
                onClick={async () => {
                  await invoke("lock_albums");
                  onChanged();
                  refresh();
                }}
                className="rounded-md border border-edge px-3 py-1 text-[12px] hover:border-ink-faint"
              >
                Zablokuj
              </button>
            </div>
          </>
        ) : (
          <button
            onClick={() => requestUnlock(album.id)}
            className="rounded-md bg-accent px-3 py-1 text-[12px] text-white hover:bg-accent-hover"
          >
            Odblokuj hasłem
          </button>
        )}
      </div>
      {changing && unlocked && (
        <div className="mt-2 flex items-center gap-2">
          <input
            type="password"
            value={newPass}
            onChange={(e) => setNewPass(e.target.value)}
            placeholder="Nowe hasło (min. 4 znaki)"
            className="rounded-md border border-edge bg-app px-2.5 py-1 text-[12px] outline-none focus:border-accent"
          />
          <button
            onClick={async () => {
              try {
                await invoke("change_album_password", {
                  albumId: album.id,
                  newPassword: newPass,
                });
                toast("Hasło zmienione — pliki przeszyfrowane");
                setChanging(false);
                setNewPass("");
              } catch (e) {
                toast(String(e));
              }
            }}
            disabled={newPass.length < 4}
            className="rounded-md bg-accent px-3 py-1 text-[12px] text-white disabled:opacity-50"
          >
            Zapisz
          </button>
        </div>
      )}
    </div>
  );
}

export function AlbumsView() {
  const [albums, setAlbums] = useState<Album[] | null>(null);
  const [open, setOpen] = useState<Album | null>(null);
  const [creating, setCreating] = useState(false);
  const [unlockedIds, setUnlockedIds] = useState<number[]>([]);
  const { resetQuery } = useLibrary();
  const unlockRequest = useApp((s) => s.unlockRequest);

  const load = async () => {
    setAlbums(await invoke<Album[]>("list_albums"));
    setUnlockedIds(await invoke<number[]>("unlocked_albums"));
  };
  useEffect(() => {
    load();
  }, [unlockRequest]); // odśwież stan blokad po zamknięciu promptu

  function openAlbum(album: Album) {
    setOpen(album);
    if (album.type === "folder" && album.folder_path) {
      resetQuery({ parent: album.folder_path, recursive: true });
    } else {
      resetQuery({ parent: null, album_id: album.id });
    }
  }

  if (open) {
    const unlocked = unlockedIds.includes(open.id);
    return (
      <div className="flex h-full flex-col">
        {open.type === "protected" ? (
          <ProtectedAlbumBar
            album={open}
            unlocked={unlocked}
            onBack={() => {
              setOpen(null);
              resetQuery();
              load();
            }}
            onChanged={load}
          />
        ) : (
          <div className="flex items-center gap-3 border-b border-edge bg-surface px-4 py-2 text-[13px]">
            <button
              onClick={() => {
                setOpen(null);
                resetQuery();
                load();
              }}
              className="text-ink-dim hover:text-ink"
            >
              ← Albumy
            </button>
            <span className="font-medium">{open.name}</span>
            <span className="text-ink-faint">
              {open.type === "folder" ? `folder: ${open.folder_path}` : "album wirtualny"}
            </span>
          </div>
        )}
        <div className="min-h-0 flex-1">
          <Grid />
        </div>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-4xl p-6">
      <div className="mb-4 flex items-center gap-3">
        <h2 className="text-lg font-semibold tracking-tight">Albumy</h2>
        <button
          onClick={() => setCreating(!creating)}
          className="ml-auto rounded-md bg-accent px-4 py-1.5 text-[13px] font-medium text-white hover:bg-accent-hover"
        >
          + Nowy album
        </button>
      </div>

      {creating && (
        <CreateForm
          onCreated={() => {
            setCreating(false);
            load();
          }}
        />
      )}

      {albums && albums.length === 0 && !creating ? (
        <EmptyState
          icon="📚"
          title="Brak albumów"
          description="Utwórz album, a potem dodawaj do niego pliki zaznaczając je w siatce (Ctrl+klik)."
        />
      ) : (
        <div className="grid grid-cols-3 gap-4 lg:grid-cols-4">
          {albums?.map((album) => (
            <div
              key={album.id}
              onClick={() => openAlbum(album)}
              className="group cursor-pointer overflow-hidden rounded-lg border border-edge bg-surface transition-colors hover:border-ink-faint"
            >
              <div className="relative aspect-square bg-raised">
                {album.type === "protected" && !unlockedIds.includes(album.id) ? (
                  <div className="flex h-full items-center justify-center text-4xl opacity-50">
                    🔒
                  </div>
                ) : album.cover ? (
                  <img
                    src={convertFileSrc(album.cover, "thumb")}
                    className="h-full w-full object-cover"
                  />
                ) : (
                  <div className="flex h-full items-center justify-center text-4xl opacity-30">
                    {album.type === "protected" ? "🔓" : "📚"}
                  </div>
                )}
                <button
                  onClick={async (e) => {
                    e.stopPropagation();
                    if (!window.confirm(`Usunąć album „${album.name}"?`)) return;
                    try {
                      await invoke("delete_album", { id: album.id });
                    } catch (err) {
                      useApp.getState().toast(String(err));
                    }
                    load();
                  }}
                  className="absolute right-2 top-2 hidden rounded-md bg-black/60 px-2 py-1 text-[11px] text-white hover:bg-danger group-hover:block"
                >
                  Usuń
                </button>
              </div>
              <div className="px-3 py-2">
                <div className="truncate text-[13px] font-medium">
                  {album.type === "protected" && "🔒 "}
                  {album.name}
                </div>
                <div className="text-[11px] text-ink-faint">
                  {album.count} plików ·{" "}
                  {album.type === "folder"
                    ? "folderowy"
                    : album.type === "protected"
                      ? "chroniony"
                      : "wirtualny"}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
