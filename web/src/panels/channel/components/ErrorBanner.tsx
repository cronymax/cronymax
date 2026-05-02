import { useEffect, useState } from "react";
import type { AppEvent } from "@/types/events";

const STORAGE_KEY = "channel.dismissed-errors";

function loadDismissed(): Set<string> {
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY);
    if (!raw) return new Set();
    return new Set(JSON.parse(raw) as string[]);
  } catch {
    return new Set();
  }
}

function saveDismissed(ids: Set<string>) {
  try {
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify([...ids]));
  } catch {
    // ignore
  }
}

interface Props {
  errors: Array<Extract<AppEvent, { kind: "error" }>>;
}

export function ErrorBanners({ errors }: Props) {
  const [dismissed, setDismissed] = useState<Set<string>>(() =>
    loadDismissed(),
  );

  useEffect(() => {
    saveDismissed(dismissed);
  }, [dismissed]);

  const visible = errors.filter((e) => !dismissed.has(e.id));
  if (visible.length === 0) return null;

  return (
    <div className="space-y-1 px-2 py-1">
      {visible.map((e) => (
        <div
          key={e.id}
          className="flex items-start gap-2 rounded border border-red-500/40 bg-red-900/30 px-2 py-1 text-xs text-red-200"
        >
          <div className="flex-1">
            <div className="font-mono text-[10px] opacity-70">
              {e.payload.scope} · {e.payload.code}
            </div>
            <div>{e.payload.message}</div>
          </div>
          <button
            type="button"
            className="rounded px-1 text-red-200/80 hover:text-red-100"
            onClick={() =>
              setDismissed((prev) => {
                const next = new Set(prev);
                next.add(e.id);
                return next;
              })
            }
            aria-label="Dismiss"
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
