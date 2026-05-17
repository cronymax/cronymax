import { Check, ShieldAlert, ShieldCheck, X } from "lucide-react";
import { useEffect, useState } from "react";
import { Alert, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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

const TRUST_VARIANT: Record<TrustLevel, "default" | "destructive" | "outline"> = {
  ask: "outline",
  autopilot: "default",
  bypass: "destructive",
};

export function ApprovalCard({ runId, reviewId, toolName, args, onAllow, onDeny }: Props) {
  const category = toolName.split("_")[0] ?? toolName;
  const [trust, setTrust] = useState<TrustLevel>(() => loadTrustMap()[category] ?? "ask");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    const level = loadTrustMap()[category] ?? "ask";
    setTrust(level);
  }, [category]);

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
    <Alert className="mx-3 mb-1 text-xs">
      <ShieldAlert />
      <AlertTitle className="flex items-center justify-between gap-2">
        <span className="flex items-center gap-2">
          <span>Tool approval required</span>
          <Badge variant={TRUST_VARIANT[trust]}>{TRUST_LABELS[trust]}</Badge>
        </span>
        <span className="font-mono text-xs font-normal text-muted-foreground">
          category: <span className="text-foreground">{category}</span>
        </span>
      </AlertTitle>

      <div className="mt-2 flex flex-col gap-2">
        <div>
          <div className="mb-1 font-medium text-foreground">{toolName}</div>
          <pre className="max-h-[120px] overflow-y-auto whitespace-pre-wrap break-all rounded bg-muted px-2 py-1 font-mono text-xs text-muted-foreground">
            {truncateJson(args)}
          </pre>
        </div>

        <div className="flex items-center gap-2">
          <Button size="sm" onClick={handleAllow}>
            <Check data-icon="inline-start" />
            Allow
          </Button>
          <Button size="sm" variant="destructive" onClick={handleDeny}>
            <X data-icon="inline-start" />
            Deny
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="ml-auto"
            onClick={() => void handleTrustAlways()}
            disabled={saving}
          >
            <ShieldCheck data-icon="inline-start" />
            Trust &quot;{category}&quot; always
          </Button>
        </div>
      </div>
    </Alert>
  );
}
