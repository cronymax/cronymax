import { useCallback, useState } from "react";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
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

  return (
    <Tabs
      value={tab}
      onValueChange={(v) => setTab(v as Tab)}
      className="flex h-screen flex-col bg-background text-foreground"
    >
      {/* ── Header ──────────────────────────────────────────────────────── */}
      <div className="flex shrink-0 items-center justify-between px-3 pb-1 pt-3">
        <span className="text-sm font-semibold">Activity</span>
        <Button
          variant="ghost"
          size="icon"
          className="h-6 w-6"
          onClick={() => shells.browser.shell.popover_close({}).catch(() => undefined)}
        >
          ×
        </Button>
      </div>

      {/* ── Filter tabs ─────────────────────────────────────────────────── */}
      <TabsList className="mx-3 mb-2 shrink-0 justify-start border-b border-border rounded-none h-auto bg-transparent px-0 pb-2">
        <TabsTrigger
          value="all"
          className="rounded px-3 py-1 text-xs data-[state=active]:bg-primary/20 data-[state=active]:text-primary"
        >
          All
        </TabsTrigger>
        <TabsTrigger
          value="live"
          className="rounded px-3 py-1 text-xs data-[state=active]:bg-primary/20 data-[state=active]:text-primary"
        >
          Live
        </TabsTrigger>
        <TabsTrigger
          value="needs_review"
          className="rounded px-3 py-1 text-xs data-[state=active]:bg-primary/20 data-[state=active]:text-primary"
        >
          Needs Review
          {pendingCount > 0 && (
            <span className="ml-1.5 rounded-full bg-amber-500 px-1.5 py-0.5 text-xs font-bold text-white">
              {pendingCount}
            </span>
          )}
        </TabsTrigger>
      </TabsList>

      {/* ── Tree ────────────────────────────────────────────────────────── */}
      <TabsContent value="all" className="flex-1 overflow-hidden">
        <ActivityTree groups={groups} reviews={reviews} onReviewResolved={onReviewResolved} />
      </TabsContent>
      <TabsContent value="live" className="flex-1 overflow-hidden">
        <ActivityTree groups={groups} reviews={reviews} onReviewResolved={onReviewResolved} />
      </TabsContent>
      <TabsContent value="needs_review" className="flex-1 overflow-hidden">
        <ActivityTree groups={groups} reviews={reviews} onReviewResolved={onReviewResolved} />
      </TabsContent>
    </Tabs>
  );
}
