/**
 * Zod schemas mirroring the C++ payload shapes. These are the runtime
 * counterparts to the channel registry in bridge_channels.ts.
 *
 * Keep the field names and types in sync with the producers in
 * app/browser/bridge_handler.cc and app/workspace/space_store.cc.
 */

import { z } from "zod";

export const EmptySchema = z.unknown().optional();

// ── space ────────────────────────────────────────────────────────────
export const SpaceSchema = z.object({
  id: z.string(),
  name: z.string(),
  root_path: z.string(),
  /** FK to ~/.cronymax/profiles/<id>.yaml (defaults to "default"). */
  profile_id: z.string().default("default"),
  /** Set to true for the currently active space (from space.list). */
  active: z.boolean().optional(),
  last_active: z.union([z.string(), z.number()]).optional(),
});
export type Space = z.infer<typeof SpaceSchema>;

// ── terminal ─────────────────────────────────────────────────────────
export const TerminalRowSchema = z.object({
  id: z.string(),
  name: z.string(),
});
export type TerminalRow = z.infer<typeof TerminalRowSchema>;

export const TerminalIdPayloadSchema = z.object({ id: z.string() });

export const TerminalInputPayloadSchema = z.object({
  id: z.string(),
  data: z.string(),
});

export const TerminalOutputPayloadSchema = z.object({
  id: z.string(),
  data: z.string(),
});

export const TerminalExitPayloadSchema = z.object({
  id: z.string(),
  code: z.number().optional(),
});

// ── browser tabs ─────────────────────────────────────────────────────
// Wire format from app/browser/browser_manager.cc / main_window.cc:
//   {id: number, url: string, title: string, is_pinned: boolean}
export const BrowserTabSchema = z.object({
  id: z.number(),
  url: z.string().optional(),
  title: z.string().optional(),
  is_pinned: z.boolean().optional(),
});
export type BrowserTab = z.infer<typeof BrowserTabSchema>;

export const TabsListResponseSchema = z.object({
  tabs: z.array(BrowserTabSchema),
  active_tab_id: z.number().optional(),
});

// Legacy: BrowserManager-era tabs use number ids. New TabManager world uses
// string ids ("tab-N"). During the arc-style-tab-cards transition both
// shapes are accepted on shell.tab_switch / shell.tab_close.
export const TabIdPayloadSchema = z.object({
  id: z.union([z.number(), z.string()]),
});

// ── arc-style-tab-cards (Phase 2) ────────────────────────────────────
// Discriminator for the new tab system. Mirrors `cronymax::TabKind`.
export const TabKindEnum = z.enum(["web", "terminal", "chat", "agent", "graph"]);
export type TabKind = z.infer<typeof TabKindEnum>;

const tabSummaryBase = {
  id: z.string(),
  displayName: z.string(),
};

export const TabSummarySchema = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("web"),
    ...tabSummaryBase,
    url: z.string().optional(),
    favicon: z.string().optional(),
  }),
  z.object({ kind: z.literal("terminal"), ...tabSummaryBase }),
  z.object({ kind: z.literal("chat"), ...tabSummaryBase }),
  z.object({ kind: z.literal("agent"), ...tabSummaryBase }),
  z.object({ kind: z.literal("graph"), ...tabSummaryBase }),
]);
export type TabSummary = z.infer<typeof TabSummarySchema>;

export const ToolbarStateSchema = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("web"),
    url: z.string().optional(),
    title: z.string().optional(),
    canGoBack: z.boolean().optional(),
    canGoForward: z.boolean().optional(),
    isLoading: z.boolean().optional(),
  }),
  z.object({
    kind: z.literal("terminal"),
    name: z.string(),
    cwd: z.string().optional(),
    state: z.enum(["idle", "running", "exited"]).optional(),
    shell: z.string().optional(),
  }),
  z.object({
    kind: z.literal("chat"),
    name: z.string(),
    model: z.string().optional(),
    messageCount: z.number().optional(),
  }),
  z.object({
    kind: z.literal("agent"),
    name: z.string(),
    runState: z.enum(["idle", "running", "paused", "error"]).optional(),
  }),
  z.object({
    kind: z.literal("graph"),
    name: z.string(),
    historyDepth: z.number().optional(),
  }),
]);
export type ToolbarState = z.infer<typeof ToolbarStateSchema>;

export const ShellTabOpenSingletonPayloadSchema = z.object({
  kind: TabKindEnum,
});
export const ShellTabOpenSingletonResponseSchema = z.object({
  tabId: z.string(),
  created: z.boolean(),
});

// native-title-bar: one button → one new tab. Single channel for all kinds.
export const ShellNewTabKindPayloadSchema = z.object({
  kind: z.enum(["web", "terminal", "chat"]),
});
export const ShellNewTabKindResponseSchema = z.object({
  tabId: z.string(),
  kind: z.string(),
});

export const TabSetToolbarStatePayloadSchema = z.object({
  tabId: z.string(),
  state: ToolbarStateSchema,
});

export const TabSetChromeThemePayloadSchema = z.object({
  tabId: z.string(),
  color: z.string().nullable(),
});

export const TabsListSnapshotSchema = z.object({
  tabs: z.array(TabSummarySchema),
  activeTabId: z.string().nullable(),
});

export const TabActivatedEventSchema = z.object({ tabId: z.string() });

// ── shell ────────────────────────────────────────────────────────────
export const ShellNavigatePayloadSchema = z.object({ url: z.string() });

export const ShellPopoverOpenPayloadSchema = z.object({ url: z.string() });

// ── shell broadcast events ───────────────────────────────────────────
export const TabCreatedSchema = BrowserTabSchema;
export const TabClosedSchema = z.object({ id: z.number() });
export const TabTitleChangedSchema = z.object({
  id: z.number(),
  title: z.string(),
});
export const TabUrlChangedSchema = z.object({
  id: z.number(),
  url: z.string(),
});
export const ActiveTabChangedSchema = z.object({ id: z.number() });
export const SpaceChangedSchema = z.object({
  id: z.string(),
  name: z.string(),
});

// ── terminal list response ───────────────────────────────────────────
export const TerminalListResponseSchema = z.object({
  active: z.string().nullable().optional(),
  items: z.array(TerminalRowSchema),
});

// ── terminal blocks (persisted command rows) ─────────────────────────
// Wire format from app/browser/bridge_handler.cc terminal.block_save / blocks_load.
export const TerminalBlockSchema = z.object({
  id: z.number().optional(),
  command: z.string(),
  output: z.string(),
  exit_code: z.number(),
  started_at: z.number(),
  ended_at: z.number(),
});
export type TerminalBlock = z.infer<typeof TerminalBlockSchema>;

export const TerminalBlockSavePayloadSchema = z.object({
  command: z.string(),
  output: z.string(),
  exit_code: z.number(),
  started_at: z.number(),
  ended_at: z.number(),
  space_id: z.string().optional(),
});

export const TerminalBlocksLoadPayloadSchema = z.object({
  space_id: z.string().optional(),
});

export const TerminalRunPayloadSchema = z.object({
  id: z.string(),
  command: z.string(),
});

// ── agent ────────────────────────────────────────────────────────────
export const AgentTaskFromCommandPayloadSchema = z.object({
  action: z.string(),
  command: z.string(),
  output: z.string(),
  exit_code: z.number(),
});

// ── tools ────────────────────────────────────────────────────────────
export const ToolExecPayloadSchema = z.object({
  name: z.string(),
  input: z.string(),
});
export const ToolExecResultSchema = z.object({
  ok: z.boolean(),
  output: z.string().optional(),
  error: z.string().optional(),
});

// ── permission ────────────────────────────────────────────────────────
export const PermissionRequestSchema = z.object({
  request_id: z.string(),
  prompt: z.string(),
});
export const PermissionRespondPayloadSchema = z.object({
  request_id: z.string(),
  decision: z.enum(["allow", "deny"]),
});

// ── llm config ────────────────────────────────────────────────────────
export const LlmConfigSchema = z.object({
  base_url: z.string(),
  api_key: z.string(),
});
export const LlmConfigSetPayloadSchema = z.object({
  base_url: z.string(),
  api_key: z.string(),
});

// ── agent task ────────────────────────────────────────────────────────
export const AgentRunPayloadSchema = z.object({
  task: z.string(),
});

// ── theme (refine-ui-theme-layout) ─────────────────────────────────────
export const ThemeModeSchema = z.enum(["system", "light", "dark"]);
export type ThemeMode = z.infer<typeof ThemeModeSchema>;
export const ThemeResolvedSchema = z.enum(["light", "dark"]);
export type ThemeResolved = z.infer<typeof ThemeResolvedSchema>;

const ThemeChromeSchema = z.object({
  bg_body: z.string(),
  bg_base: z.string(),
  bg_float: z.string(),
  bg_mask: z.string(),
  border: z.string(),
  text_title: z.string(),
  text_caption: z.string(),
});

export const ThemeGetResponseSchema = z.object({
  mode: ThemeModeSchema,
  resolved: ThemeResolvedSchema,
});
export const ThemeSetPayloadSchema = z.object({ mode: ThemeModeSchema });
export const ThemeChangedPayloadSchema = z.object({
  mode: ThemeModeSchema,
  resolved: ThemeResolvedSchema,
  chrome: ThemeChromeSchema,
});

// ── shell.settings_popover_open (refine-ui-theme-layout) ──────────────
export const ShellSettingsPopoverOpenPayloadSchema = EmptySchema;
export const ShellSettingsPopoverOpenResponseSchema = EmptySchema;

// ── agent-event-bus / inbox (re-exported) ─────────────────────────────
export * from "./events";
