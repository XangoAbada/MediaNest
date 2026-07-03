import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useApp } from "../stores/app";
import { useLibrary } from "../stores/library";

/** Globalny modal odblokowania chronionego albumu (app.unlockRequest). */
export function PasswordPrompt() {
  const { unlockRequest, requestUnlock, toast } = useApp();
  const refresh = useLibrary((s) => s.refresh);
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setPassword("");
    setError(null);
  }, [unlockRequest]);

  if (unlockRequest === null) return null;

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      await invoke("unlock_album", { id: unlockRequest, password });
      toast("Album odblokowany do końca sesji");
      requestUnlock(null);
      refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/60"
      onClick={() => requestUnlock(null)}
    >
      <div
        className="w-96 rounded-2xl border border-edge bg-raised p-6 shadow-lg"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-1 flex items-center gap-2 text-[15px] font-semibold">
          <span>🔒</span> Album chroniony
        </div>
        <p className="mb-4 text-[13px] text-ink-dim">
          Podaj hasło, aby odblokować zawartość na czas tej sesji. Po zamknięciu
          aplikacji album zablokuje się ponownie.
        </p>
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") submit();
            if (e.key === "Escape") requestUnlock(null);
          }}
          autoFocus
          placeholder="Hasło"
          className="w-full rounded-md border border-edge bg-app px-3 py-2 text-[13px] outline-none focus:border-accent"
        />
        {error && <p className="mt-2 text-[13px] text-danger">{error}</p>}
        <div className="mt-4 flex justify-end gap-2">
          <button
            onClick={() => requestUnlock(null)}
            className="rounded-md border border-edge px-4 py-1.5 text-[13px] hover:border-ink-faint"
          >
            Anuluj
          </button>
          <button
            onClick={submit}
            disabled={busy || !password}
            className="rounded-md bg-accent px-4 py-1.5 text-[13px] font-medium text-white hover:bg-accent-hover disabled:opacity-50"
          >
            Odblokuj
          </button>
        </div>
      </div>
    </div>
  );
}
