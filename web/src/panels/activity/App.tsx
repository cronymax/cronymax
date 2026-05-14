import { useCallback, useState } from "react";
import { shells } from "@/shells/bridge";
import { ActivityTree } from "./ActivityTree";
import { useActivityFeed } from "./useActivityFeed";

type Tab = "all" | "live" | "needs_review";

export function App() {
  const [tab, setTab] = useState<Tab>("all");

  // Re-render after approval to refresh pending state.
  const [, setRevision] = useState(0);
  const onReviewResolved = useCallback(() => setRevision((r) => r + 1), []);

  const { chatGroups, flowGroups, pendingCount, reviews } = useActivityFeed(tab);

  const groups = { chatGroups, flowGroups, pendingCount };

  const tabClass = (t: Tab) =>
    `px-3 py-1 text-xs rounded cursor-pointer transition-colors ${
      tab === t
        ? "bg-cronymax-accent/20 text-cronymax-accent font-medium"
        : "text-cronymax-caption hover:text-cronymax-title"
    }`;

  return (
    <div className="flex flex-col h-screen bg-cronymax-base text-cronymax-title">
      {/* ── Header ──────────────────────────────────────────────────────── */}
      <div className="flex items-center justify-between px-3 pt-3 pb-1 shrink-0">
        <span className="text-sm font-semibold">Activity</span>
        <button
          type="button"
          className="text-cronymax-caption hover:text-cronymax-title text-xs px-2 py-1 rounded hover:bg-cronymax-hover"
          onClick={() => shells.browser.shell.popover_close({}).catch(() => undefined)}
        >
          ×
        </button>
      </div>

      {/* ── Filter tabs ─────────────────────────────────────────────────── */}
      <div className="flex items-center gap-1 px-3 pb-2 shrink-0 border-b border-cronymax-border">
        <button type="button" className={tabClass("all")} onClick={() => setTab("all")}>
          All
        </button>
        <button type="button" className={tabClass("live")} onClick={() => setTab("live")}>
          Live
        </button>
        <button type="button" className={tabClass("needs_review")} onClick={() => setTab("needs_review")}>
          Needs Review
          {pendingCount > 0 && (
            <span className="ml-1.5 rounded-full bg-amber-500 text-white text-[10px] font-bold px-1.5 py-0.5">
              {pendingCount}
            </span>
          )}
        </button>
      </div>

      {/* ── Tree ────────────────────────────────────────────────────────── */}
      <ActivityTree groups={groups} reviews={reviews} onReviewResolved={onReviewResolved} />
    </div>
  );
}
