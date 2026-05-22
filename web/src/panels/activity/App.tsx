import { useCallback, useState } from "react";
import { PanelWindowHeader } from "@/components/PanelWindowHeader";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Heading } from "../../components/ui/typography";
import { ActivityTree } from "./ActivityTree";
import { useActivityFeed } from "./useActivityFeed";

type Tab = "all" | "live" | "needs_review";

export function App() {
  const [tab, setTab] = useState<Tab>("all");

  // Re-render after approval to refresh pending state.
  const [, setRevision] = useState(0);
  const onReviewResolved = useCallback(() => setRevision((r) => r + 1), []);

  const { chatRoots, flowRoots, pendingCount, reviews } = useActivityFeed(tab);

  const groups = { chatRoots, flowRoots, pendingCount };

  return (
    <Tabs
      value={tab}
      onValueChange={(v) => setTab(v as Tab)}
      className="flex h-screen flex-col bg-background text-foreground"
    >
      {/* ── Header ──────────────────────────────────────────────────────── */}
      <PanelWindowHeader className="flex shrink-0 items-center border-b border-border bg-card px-4 py-2">
        <Heading>Activity</Heading>
      </PanelWindowHeader>

      {/* ── Filter tabs ─────────────────────────────────────────────────── */}
      <TabsList className="mx-4">
        <TabsTrigger value="all" className="gap-1.5">
          All
        </TabsTrigger>
        <TabsTrigger value="live" className="gap-1.5">
          Live
        </TabsTrigger>
        <TabsTrigger value="needs_review" className="gap-1.5">
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
