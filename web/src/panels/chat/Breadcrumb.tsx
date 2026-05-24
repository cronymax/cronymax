/**
 * Breadcrumb — shown at the top of a thread view.  Provides a back button
 * to return to the main chat timeline.
 *
 * supervisor-session-ux task 7.2
 */

import { ArrowLeft } from "lucide-react";
import { useCallback } from "react";
import { Button } from "@/components/ui/button";
import { useStore } from "./store";

interface Props {
  label: string;
}

export function Breadcrumb({ label }: Props) {
  const [, dispatch] = useStore();

  const handleBack = useCallback(() => {
    dispatch({ type: "setThreadView", target: null });
  }, [dispatch]);

  return (
    <div className="flex items-center gap-2 border-b border-border/60 px-4 py-2">
      <Button size="sm" variant="ghost" className="gap-1 px-2 text-sm" onClick={handleBack}>
        <ArrowLeft className="h-4 w-4" />
        Chat
      </Button>
      <span className="text-muted-foreground">/</span>
      <span className="truncate text-sm font-medium">{label}</span>
    </div>
  );
}
