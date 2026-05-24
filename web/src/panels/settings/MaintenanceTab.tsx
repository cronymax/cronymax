/**
 * MaintenanceTab — housekeeping actions for local browser-side state.
 *
 * Everything here operates on localStorage / sessionStorage only. The
 * runtime's own persisted state (runs / reviews / sessions in
 * `runtime-state.json`) is NOT touched from here — that lives in a
 * separate process and would need a dedicated runtime API to wipe.
 */

import { AlertTriangle, MessageSquare, ShieldCheck, Trash2 } from "lucide-react";
import { useCallback, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Caption } from "@/components/ui/typography";
import { shells } from "@/shells/bridge";

const TRUST_KEY = "cronymax.tool_trust";
const CHATS_LIST_KEY = "chats";
const CHAT_HISTORY_PREFIX = "chat_history_v4:";
const CHAT_HISTORY_PREFIX_V3 = "chat_history_v3:";
const CHAT_HISTORY_PREFIX_V2 = "chat_history_v2:";
const CHAT_HISTORY_PREFIX_V1 = "chat_history:";

function chatHistoryKeys(): string[] {
  const out: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const k = localStorage.key(i);
    if (!k) continue;
    if (
      k.startsWith(CHAT_HISTORY_PREFIX) ||
      k.startsWith(CHAT_HISTORY_PREFIX_V3) ||
      k.startsWith(CHAT_HISTORY_PREFIX_V2) ||
      k.startsWith(CHAT_HISTORY_PREFIX_V1)
    ) {
      out.push(k);
    }
  }
  return out;
}

interface ActionRowProps {
  icon: typeof Trash2;
  title: string;
  description: string;
  countLabel?: string;
  confirmTitle: string;
  confirmDescription: string;
  confirmCta: string;
  destructive?: boolean;
  onConfirm: () => void;
}

function ActionRow({
  icon: Icon,
  title,
  description,
  countLabel,
  confirmTitle,
  confirmDescription,
  confirmCta,
  destructive,
  onConfirm,
}: ActionRowProps) {
  const [open, setOpen] = useState(false);
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-sm">
          <Icon className="size-4 text-muted-foreground" aria-hidden="true" />
          {title}
        </CardTitle>
        <CardDescription className="text-xs">{description}</CardDescription>
      </CardHeader>
      <CardContent className="flex items-center justify-between gap-3">
        <Caption className="text-xs text-muted-foreground">{countLabel ?? ""}</Caption>
        <Button size="sm" variant={destructive ? "destructive" : "outline"} onClick={() => setOpen(true)}>
          {confirmCta}
        </Button>
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogContent className="w-[380px]">
            <DialogHeader>
              <DialogTitle className="text-sm">{confirmTitle}</DialogTitle>
              <DialogDescription className="text-xs">
                {confirmDescription}
                <br />
                <span className="mt-1 inline-block font-medium text-foreground">
                  The app will relaunch once cleaning finishes.
                </span>
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="outline" size="sm" onClick={() => setOpen(false)}>
                Cancel
              </Button>
              <Button
                size="sm"
                variant={destructive ? "destructive" : "default"}
                onClick={() => {
                  onConfirm();
                  shells.browser.shell.relaunch().catch(() => undefined);
                  setOpen(false);
                }}
              >
                {confirmCta}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </CardContent>
    </Card>
  );
}

export function MaintenanceTab() {
  const [, setTick] = useState(0);
  const refresh = useCallback(() => setTick((n) => n + 1), []);

  const trustCount = (() => {
    try {
      const raw = localStorage.getItem(TRUST_KEY);
      if (!raw) return 0;
      return Object.keys(JSON.parse(raw) as Record<string, unknown>).length;
    } catch {
      return 0;
    }
  })();

  const historyKeys = chatHistoryKeys();
  const chatListCount = (() => {
    try {
      const raw = localStorage.getItem(CHATS_LIST_KEY);
      if (!raw) return 0;
      return (JSON.parse(raw) as unknown[]).length;
    } catch {
      return 0;
    }
  })();

  const clearTrust = useCallback(() => {
    try {
      localStorage.removeItem(TRUST_KEY);
    } catch {
      /* ignore */
    }
    refresh();
  }, [refresh]);

  const clearChatHistory = useCallback(() => {
    try {
      for (const k of chatHistoryKeys()) localStorage.removeItem(k);
      localStorage.removeItem(CHATS_LIST_KEY);
    } catch {
      /* ignore */
    }
    refresh();
  }, [refresh]);

  const clearAllLocal = useCallback(() => {
    try {
      localStorage.clear();
      sessionStorage.clear();
    } catch {
      /* ignore */
    }
    refresh();
  }, [refresh]);

  return (
    <div className="flex flex-col gap-3 p-4">
      <Caption className="text-xs text-muted-foreground">
        Local browser-side state only. Runtime state (runs / reviews) lives in a separate process and is not affected.
      </Caption>

      <ActionRow
        icon={ShieldCheck}
        title="Forget tool trust"
        description="Per-tool 'Trust always' decisions saved when you approved a tool category. Forgetting them returns to Ask for each category."
        countLabel={`${trustCount} categor${trustCount === 1 ? "y" : "ies"} trusted`}
        confirmTitle="Forget tool trust?"
        confirmDescription="Future tool calls in categories you previously trusted will surface the approval card again."
        confirmCta="Forget trust"
        onConfirm={clearTrust}
      />

      <ActionRow
        icon={MessageSquare}
        title="Clear chat history"
        description="Removes all chat blocks and the chat list from local storage. Does not touch runtime runs."
        countLabel={`${chatListCount} chat${chatListCount === 1 ? "" : "s"} · ${historyKeys.length} history blob${historyKeys.length === 1 ? "" : "s"}`}
        confirmTitle="Clear all chat history?"
        confirmDescription="This deletes every chat in this app instance and cannot be undone."
        confirmCta="Delete chats"
        destructive
        onConfirm={clearChatHistory}
      />

      <ActionRow
        icon={Trash2}
        title="Wipe all local data"
        description="Clears every key in localStorage and sessionStorage for this origin. Use as a last resort."
        confirmTitle="Wipe ALL local data?"
        confirmDescription="Trust decisions, chat history, prompt drafts, model selections, and any other locally cached preference will be gone."
        confirmCta="Wipe everything"
        destructive
        onConfirm={clearAllLocal}
      />

      <Card className="border-amber-500/40 bg-amber-500/5">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-sm text-amber-600 dark:text-amber-400">
            <AlertTriangle className="size-4" aria-hidden="true" />
            Runtime data
          </CardTitle>
          <CardDescription className="text-xs">
            Runs, reviews, agent sessions, and persisted permission state live in <code>runtime-state.json</code>, owned
            by the runtime process. Wipe by quitting the app and deleting the file under{" "}
            <code>~/Library/Application Support/app.cronymax/cronymax/Profiles/default/</code>.
          </CardDescription>
        </CardHeader>
      </Card>
    </div>
  );
}
