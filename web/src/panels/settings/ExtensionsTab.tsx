/**
 * Settings → Extensions tab.
 *
 * Lists every installed extension (enabled or not), with per-row
 * enable/disable + uninstall, an expandable details panel, and an
 * "Install…" action that opens the native folder/`.cmx` picker.
 *
 * Data comes from the `extension.*` runtime control requests
 * (`extensionRegistry`); the list refetches on the `extensions/contributions`
 * topic and on `runtime.reconnected`, mirroring the activity-bar rail.
 */
import {
  Blocks,
  ChevronDown,
  ChevronRight,
  FolderInput,
  Loader2,
  Power,
  ScrollText,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Caption } from "@/components/ui/typography";
import { cn } from "@/lib/utils";
import { browser, runtime, shells } from "@/shells/bridge";
import { extensionRegistry, type InstalledExtension } from "@/shells/runtime";
import { ExtensionLogsView } from "./ExtensionLogsView";

// ── helpers ─────────────────────────────────────────────────────────────────

/** Close an extension's open operation views (main-area tab → back to the
 *  latest chat; right dock → collapsed), mirroring the activity-bar rail's
 *  "Disable" path. A no-op when the extension has nothing open. Best-effort:
 *  a failure here shouldn't block the enable-flag change. */
function closeExtensionViews(extId: string): Promise<unknown> {
  return shells.browser.shell.close_extension_views({ ext_id: extId }).catch(() => undefined);
}

/** Strip the control-error envelope ("... failed [N]: " / "install failed: ")
 *  down to the human-readable tail. */
function humanizeError(e: unknown): string {
  const msg = e instanceof Error ? e.message : String(e);
  return msg.replace(/^.*?failed(?: \[\d+\])?:\s*/i, "").trim() || msg;
}

/** "2 commands · 1 view · 1 renderer" — only non-zero kinds, omitted entirely
 *  when the extension contributes nothing. */
function summarizeContributes(c: InstalledExtension["contributes"]): string {
  const parts: string[] = [];
  const add = (n: number, one: string, many: string) => {
    if (n > 0) parts.push(`${n} ${n === 1 ? one : many}`);
  };
  add(c.agent_providers, "provider", "providers");
  add(c.content_renderers, "renderer", "renderers");
  add(c.sidebar_views, "view", "views");
  add(c.commands, "command", "commands");
  add(c.config_pages, "config page", "config pages");
  if (c.has_config_schema) parts.push("config schema");
  return parts.join(" · ");
}

// ── row ───────────────────────────────────────────────────────────────────

function ExtensionRow({
  ext,
  busy,
  onToggle,
  onUninstall,
  onLogs,
}: {
  ext: InstalledExtension;
  busy: boolean;
  onToggle: (ext: InstalledExtension) => void;
  onUninstall: (ext: InstalledExtension) => void;
  onLogs: (ext: InstalledExtension) => void;
}) {
  const [open, setOpen] = useState(false);
  const summary = summarizeContributes(ext.contributes);

  return (
    <div className={cn("rounded-lg border border-border bg-card", !ext.enabled && "bg-muted/30")}>
      <div className="flex items-start gap-3 p-3">
        <button
          type="button"
          aria-label={open ? "Collapse details" : "Expand details"}
          onClick={() => setOpen((o) => !o)}
          className="mt-0.5 flex size-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
        >
          {open ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
        </button>

        {/* Dim the descriptive column when disabled so the row reads as "off";
            the action button stays full-strength so Enable is obvious. */}
        <div className={cn("min-w-0 flex-1", !ext.enabled && "opacity-55")}>
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-medium text-foreground">{ext.name}</span>
            <span className="shrink-0 text-xs text-muted-foreground">v{ext.version}</span>
            {ext.enabled ? (
              ext.active ? (
                <span className="shrink-0 rounded-full bg-primary/15 px-1.5 py-0.5 text-[10px] font-medium text-primary">
                  Active
                </span>
              ) : (
                <span className="shrink-0 rounded-full bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                  Enabled
                </span>
              )
            ) : ext.disabled_reason === "crash" ? (
              <span
                className="shrink-0 rounded-full bg-destructive/15 px-1.5 py-0.5 text-[10px] font-medium text-destructive"
                title="Auto-disabled after repeated crashes. Enable to try again."
              >
                Disabled (crashed)
              </span>
            ) : (
              <span className="shrink-0 rounded-full bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                Disabled
              </span>
            )}
          </div>
          <div className="mt-0.5 truncate text-xs text-muted-foreground">{ext.publisher}</div>
          {ext.description && <p className="mt-1 line-clamp-2 text-xs text-muted-foreground">{ext.description}</p>}
          {summary && <div className="mt-1 text-[11px] text-muted-foreground">{summary}</div>}
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          <Button
            variant="ghost"
            size="icon"
            onClick={() => onLogs(ext)}
            title="View logs"
            className="text-muted-foreground hover:text-foreground"
          >
            <ScrollText className="size-3.5" />
          </Button>
          <Button
            variant={ext.enabled ? "outline" : "default"}
            size="sm"
            disabled={busy}
            onClick={() => onToggle(ext)}
            title={ext.enabled ? "Disable extension" : "Enable extension"}
          >
            {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Power className="size-3.5" />}
            {ext.enabled ? "Disable" : "Enable"}
          </Button>
          <Button
            variant="ghost"
            size="icon"
            disabled={busy}
            onClick={() => onUninstall(ext)}
            title="Uninstall extension"
            className="text-muted-foreground hover:text-destructive"
          >
            <Trash2 className="size-3.5" />
          </Button>
        </div>
      </div>

      {open && (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 border-t border-border px-3 py-2 text-[11px]">
          <dt className="text-muted-foreground">ID</dt>
          <dd className="truncate font-mono text-foreground">{ext.id}</dd>
          <dt className="text-muted-foreground">Type</dt>
          <dd className="text-foreground">{ext.has_main ? "Host-backed (Node)" : "Declarative-only"}</dd>
          <dt className="text-muted-foreground">Contributes</dt>
          <dd className="text-foreground">{summary || "—"}</dd>
          <dt className="text-muted-foreground">Path</dt>
          <dd className="break-all font-mono text-foreground">{ext.ext_dir}</dd>
        </dl>
      )}
    </div>
  );
}

// ── tab ─────────────────────────────────────────────────────────────────────

export function ExtensionsTab() {
  // `null` = initial load in flight; `[]` = loaded, none installed.
  const [extensions, setExtensions] = useState<InstalledExtension[] | null>(null);
  const [busyIds, setBusyIds] = useState<ReadonlySet<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [pendingUninstall, setPendingUninstall] = useState<InstalledExtension | null>(null);
  // When set, the tab shows that extension's logs instead of the list.
  const [logsFor, setLogsFor] = useState<InstalledExtension | null>(null);

  const refetch = useCallback(async () => {
    try {
      const { extensions } = await extensionRegistry.list();
      setExtensions(extensions);
    } catch (e) {
      setError(humanizeError(e));
      setExtensions((prev) => prev ?? []);
    }
  }, []);

  useEffect(() => {
    void refetch();
    // Activate/deactivate elsewhere (rail "Disable", startup) changes the
    // active set — refetch on the same signal the rail uses.
    const offContrib = runtime.on("extensions/contributions", () => void refetch());
    const offReconnect = browser.on("runtime.reconnected", () => void refetch());
    return () => {
      offContrib?.();
      offReconnect();
    };
  }, [refetch]);

  const runAction = useCallback(
    async (id: string, fn: () => Promise<void>) => {
      setError(null);
      setBusyIds((s) => new Set(s).add(id));
      try {
        await fn();
        await refetch();
      } catch (e) {
        setError(humanizeError(e));
      } finally {
        setBusyIds((s) => {
          const next = new Set(s);
          next.delete(id);
          return next;
        });
      }
    },
    [refetch],
  );

  const onToggle = useCallback(
    (ext: InstalledExtension) =>
      void runAction(ext.id, async () => {
        // Disabling tears the extension down — close its open views first so
        // the main-area tab returns to chat and the dock collapses, matching
        // the rail's "Disable". Enabling has nothing open to close.
        if (ext.enabled) await closeExtensionViews(ext.id);
        await extensionRegistry.setEnabled(ext.id, !ext.enabled);
      }),
    [runAction],
  );

  const confirmUninstall = useCallback(() => {
    const ext = pendingUninstall;
    setPendingUninstall(null);
    if (ext)
      void runAction(ext.id, async () => {
        await closeExtensionViews(ext.id);
        await extensionRegistry.uninstall(ext.id);
      });
  }, [pendingUninstall, runAction]);

  const onInstall = useCallback(async () => {
    setError(null);
    setInstalling(true);
    try {
      const { path } = await shells.browser.shell.pick_extension_source();
      if (!path) return; // cancelled
      await extensionRegistry.install(path);
      await refetch();
    } catch (e) {
      setError(humanizeError(e));
    } finally {
      setInstalling(false);
    }
  }, [refetch]);

  // Per-extension logs view takes over the whole tab until dismissed.
  if (logsFor) {
    return <ExtensionLogsView ext={logsFor} onBack={() => setLogsFor(null)} />;
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 items-center justify-between gap-3 px-4 pt-4 pb-2">
        <Caption>Install extensions from a folder or a .cmx package. Disabled extensions stay installed.</Caption>
        <Button size="sm" disabled={installing} onClick={() => void onInstall()}>
          {installing ? <Loader2 className="size-3.5 animate-spin" /> : <FolderInput className="size-3.5" />}
          Install…
        </Button>
      </div>

      {error && (
        <div className="mx-4 mb-2 flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0" />
          <span className="min-w-0 break-words">{error}</span>
        </div>
      )}

      <div className="flex-1 space-y-2 overflow-y-auto px-4 pb-4">
        {extensions === null ? (
          <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" />
            Loading…
          </div>
        ) : extensions.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-2 py-10 text-center text-muted-foreground">
            <Blocks className="size-8 opacity-40" />
            <p className="text-sm">No extensions installed</p>
            <p className="max-w-xs text-xs">Use Install… to add one from a folder or a .cmx package.</p>
          </div>
        ) : (
          extensions.map((ext) => (
            <ExtensionRow
              key={ext.id}
              ext={ext}
              busy={busyIds.has(ext.id)}
              onToggle={onToggle}
              onUninstall={setPendingUninstall}
              onLogs={setLogsFor}
            />
          ))
        )}
      </div>

      <Dialog open={pendingUninstall !== null} onOpenChange={(open) => !open && setPendingUninstall(null)}>
        <DialogContent className="w-[360px]">
          <DialogHeader>
            <DialogTitle className="text-sm">Uninstall extension?</DialogTitle>
            <DialogDescription className="text-sm">
              {pendingUninstall
                ? `“${pendingUninstall.name}” (${pendingUninstall.id}) will be deactivated and its files removed from ~/.cronymax/extensions. This can't be undone.`
                : ""}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPendingUninstall(null)}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={confirmUninstall}>
              <Trash2 className="size-3.5" />
              Uninstall
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
