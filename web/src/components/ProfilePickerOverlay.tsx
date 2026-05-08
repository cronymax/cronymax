/**
 * ProfilePickerOverlay — appears after the user selects a folder via the
 * native folder picker (space.open_folder / "Open Folder…" titlebar menu).
 *
 * The C++ bridge emits "space.folder_picked" with { path } when a folder
 * is selected.  This overlay lets the user pick a named sandbox profile
 * and then calls space.create.
 */
import { useCallback, useEffect, useState } from "react";
import { browser } from "@/shells/bridge";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";

interface ProfileRecord {
  id: string;
  name: string;
  allow_network: boolean;
  extra_read_paths: string[];
  extra_write_paths: string[];
  extra_deny_paths: string[];
}

export function ProfilePickerOverlay() {
  const [pendingPath, setPendingPath] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<ProfileRecord[]>([]);
  const [selectedProfile, setSelectedProfile] = useState("default");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // Listen for folder selection events from the native picker.
  useBridgeEvent("space.folder_picked", ({ path }) => {
    if (!path) return; // cancelled
    setPendingPath(path);
    setSelectedProfile("default");
    setErr(null);
  });

  // Fetch profiles when the overlay opens.
  useEffect(() => {
    if (!pendingPath) return;
    browser
      .send("profiles.list")
      .then((res) => setProfiles(res))
      .catch(() => {
        setProfiles([
          {
            id: "default",
            name: "Default",
            allow_network: true,
            extra_read_paths: [],
            extra_write_paths: [],
            extra_deny_paths: [],
          },
        ]);
      });
  }, [pendingPath]);

  const handleOpen = useCallback(async () => {
    if (!pendingPath) return;
    setBusy(true);
    setErr(null);
    try {
      await browser.send("space.create", {
        root_path: pendingPath,
        profile_id: selectedProfile,
      });
      setPendingPath(null);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }, [pendingPath, selectedProfile]);

  const handleCancel = useCallback(() => {
    setPendingPath(null);
  }, []);

  if (!pendingPath) return null;

  // Shorten path for display.
  const displayPath =
    pendingPath.length > 60
      ? "…" + pendingPath.slice(pendingPath.length - 57)
      : pendingPath;

  return (
    // Full-screen backdrop.
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
      <div className="w-[420px] rounded-lg border border-cronymax-border bg-cronymax-float p-5 shadow-xl">
        <h2 className="mb-1 text-sm font-semibold text-cronymax-title">
          Open folder as workspace
        </h2>
        <p className="mb-4 break-all font-mono text-[11px] text-cronymax-caption">
          {displayPath}
        </p>

        <label className="mb-1 block text-[11px] uppercase tracking-wide text-cronymax-caption">
          Sandbox profile
        </label>
        <select
          className="mb-4 w-full rounded border border-cronymax-border bg-cronymax-base px-2 py-1 text-xs text-cronymax-title outline-none focus:border-cronymax-primary"
          value={selectedProfile}
          onChange={(e) => setSelectedProfile(e.target.value)}
          disabled={busy}
        >
          {profiles.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
              {p.id === "default" ? " (default)" : ""}
            </option>
          ))}
        </select>

        {err && <p className="mb-3 text-xs text-red-500">{err}</p>}

        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={handleCancel}
            disabled={busy}
            className="rounded border border-cronymax-border bg-cronymax-base px-3 py-1.5 text-xs text-cronymax-title hover:bg-cronymax-float disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={() => void handleOpen()}
            disabled={busy}
            className="rounded bg-cronymax-primary px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
          >
            {busy ? "Opening…" : "Open"}
          </button>
        </div>
      </div>
    </div>
  );
}
