import { useEffect, useState } from "react";
import { shells } from "@/shells/bridge";

/** Trust level for a tool category. */
type TrustLevel = "ask" | "autopilot" | "bypass";

const TRUST_STORAGE_KEY = "cronymax.tool_trust";

export function loadTrustMap(): Record<string, TrustLevel> {
  try {
    const raw = localStorage.getItem(TRUST_STORAGE_KEY);
    if (!raw) return {};
    return JSON.parse(raw) as Record<string, TrustLevel>;
  } catch {
    return {};
  }
}

function saveTrustMap(map: Record<string, TrustLevel>): void {
  try {
    localStorage.setItem(TRUST_STORAGE_KEY, JSON.stringify(map));
  } catch {
    /* ignore */
  }
}

interface Props {
  runId: string;
  reviewId: string;
  toolName: string;
  args: unknown;
  onAllow: () => void;
  onDeny: () => void;
}

function truncateJson(value: unknown, maxLen = 200): string {
  try {
    const s = JSON.stringify(value, null, 2);
    if (s.length <= maxLen) return s;
    return `${s.slice(0, maxLen)}…`;
  } catch {
    return String(value);
  }
}

const TRUST_LABELS: Record<TrustLevel, string> = {
  ask: "Ask",
  autopilot: "Autopilot",
  bypass: "Bypass",
};

export function ApprovalCard({ runId, reviewId, toolName, args, onAllow, onDeny }: Props) {
  const category = toolName.split("_")[0] ?? toolName;
  const [trust, setTrust] = useState<TrustLevel>(() => loadTrustMap()[category] ?? "ask");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    const level = loadTrustMap()[category] ?? "ask";
    setTrust(level);
  }, [reviewId, category]);

  const handleAllow = () => {
    shells.review.approve({ review_id: reviewId }).catch(() => undefined);
    onAllow();
  };

  const handleDeny = () => {
    shells.review.request_changes({ review_id: reviewId }).catch(() => undefined);
    onDeny();
  };

  const handleTrustAlways = async () => {
    setSaving(true);
    try {
      const map = loadTrustMap();
      map[category] = "autopilot";
      saveTrustMap(map);
      setTrust("autopilot");
      await shells.review.approve({ review_id: reviewId });
    } catch {
      /* ignore */
    } finally {
      setSaving(false);
    }
    onAllow();
  };

  // Suppress unused variable warning
  void runId;

  return (
    <div className="mx-3 mb-1 rounded-lg border border-amber-500/50 bg-amber-500/5 p-3 text-xs">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="font-semibold text-amber-300">Tool approval required</span>
          <span
            className={
              "rounded px-1.5 py-0.5 text-[10px] font-mono " +
              (trust === "autopilot"
                ? "bg-green-500/20 text-green-300"
                : trust === "bypass"
                  ? "bg-red-500/20 text-red-300"
                  : "bg-amber-500/20 text-amber-300")
            }
          >
            {TRUST_LABELS[trust]}
          </span>
        </div>
        <span className="font-mono text-[11px] text-cronymax-caption">
          category: <span className="text-cronymax-title">{category}</span>
        </span>
      </div>

      <div className="mb-2">
        <div className="mb-1 font-semibold text-cronymax-title">{toolName}</div>
        <pre className="max-h-[120px] overflow-y-auto rounded bg-cronymax-base px-2 py-1 font-mono text-[11px] text-cronymax-caption whitespace-pre-wrap break-all">
          {truncateJson(args)}
        </pre>
      </div>

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={handleAllow}
          className="rounded bg-green-500/80 px-3 py-1 text-xs font-medium text-white hover:bg-green-500"
        >
          Allow
        </button>
        <button
          type="button"
          onClick={handleDeny}
          className="rounded border border-red-500/50 bg-red-500/10 px-3 py-1 text-xs text-red-300 hover:bg-red-500/20"
        >
          Deny
        </button>
        <button
          type="button"
          onClick={() => void handleTrustAlways()}
          disabled={saving}
          className="ml-auto rounded border border-cronymax-border bg-cronymax-base px-3 py-1 text-xs text-cronymax-caption hover:text-cronymax-title disabled:opacity-50"
        >
          Trust &quot;{category}&quot; always
        </button>
      </div>
    </div>
  );
}
