/**
 * Channel registry — single source of truth for every C++ ↔ JS bridge
 * channel. Each entry maps a channel name to its request and response
 * Zod schemas. Hooks and the bridge itself read these for both compile-time
 * type narrowing and runtime payload validation.
 *
 * To add a new channel:
 *   1. Add the schema(s) to web/src/shared/types/.
 *   2. Add an entry below.
 *   3. Both `bridge.send("…")` and `useBridgeEvent("…")` will pick up
 *      the types automatically.
 *
 * Streaming-style channels with very high payload frequency may opt out
 * of full Zod validation by setting `fastPath: true` (see bridge.ts).
 */

import { z } from "zod";
import {
  ActiveTabChangedSchema,
  AgentTaskFromCommandPayloadSchema,
  AppEventSchema,
  BrowserTabSchema,
  EmptySchema,
  EventsAppendReq,
  EventsAppendRes,
  EventsListReq,
  EventsListRes,
  EventsSubscribeReq,
  EventsSubscribeRes,
  InboxListReq,
  InboxListRes,
  InboxSnoozeReq,
  InboxStateChangeReq,
  LlmConfigSchema,
  LlmConfigSetPayloadSchema,
  PermissionRespondPayloadSchema,
  ShellNavigatePayloadSchema,
  ShellNewTabKindPayloadSchema,
  ShellNewTabKindResponseSchema,
  ShellPopoverOpenPayloadSchema,
  ShellSettingsPopoverOpenPayloadSchema,
  ShellSettingsPopoverOpenResponseSchema,
  ShellTabOpenSingletonPayloadSchema,
  ShellTabOpenSingletonResponseSchema,
  SpaceChangedSchema,
  SpaceSchema,
  TabActivatedEventSchema,
  TabClosedSchema,
  TabCreatedSchema,
  TabIdPayloadSchema,
  TabSetChromeThemePayloadSchema,
  TabSetToolbarStatePayloadSchema,
  TabsListSnapshotSchema,
  TabTitleChangedSchema,
  TabUrlChangedSchema,
  TerminalBlockSavePayloadSchema,
  TerminalBlockSchema,
  TerminalBlocksLoadPayloadSchema,
  TerminalExitPayloadSchema,
  TerminalIdPayloadSchema,
  TerminalListResponseSchema,
  TerminalRowSchema,
  ThemeChangedPayloadSchema,
  ThemeGetResponseSchema,
  ThemeSetPayloadSchema,
  ToolExecPayloadSchema,
  ToolExecResultSchema,
} from "../types";

interface ChannelDef<Req extends z.ZodTypeAny, Res extends z.ZodTypeAny> {
  req: Req;
  res: Res;
  fastPath?: boolean;
}

function chan<Req extends z.ZodTypeAny, Res extends z.ZodTypeAny>(def: ChannelDef<Req, Res>): ChannelDef<Req, Res> {
  return def;
}

export const Channels = {
  // ── shell / browser ────────────────────────────────────────────────
  "shell.navigate": chan({ req: ShellNavigatePayloadSchema, res: EmptySchema }),
  "shell.go_back": chan({ req: EmptySchema, res: EmptySchema }),
  "shell.go_forward": chan({ req: EmptySchema, res: EmptySchema }),
  "shell.reload": chan({ req: EmptySchema, res: EmptySchema }),
  "shell.popover_open": chan({
    req: ShellPopoverOpenPayloadSchema,
    res: EmptySchema,
  }),
  "shell.popover_close": chan({ req: EmptySchema, res: EmptySchema }),
  "shell.popover_refresh": chan({ req: EmptySchema, res: EmptySchema }),
  "shell.popover_open_as_tab": chan({ req: EmptySchema, res: EmptySchema }),
  "shell.popover_navigate": chan({
    req: z.object({ url: z.string() }),
    res: EmptySchema,
  }),
  "shell.open_external": chan({
    req: z.object({ url: z.string() }),
    res: EmptySchema,
  }),
  "shell.window_drag": chan({ req: EmptySchema, res: EmptySchema }),
  "shell.tabs_list": chan({ req: EmptySchema, res: TabsListSnapshotSchema }),
  "shell.tab_new": chan({
    req: ShellNavigatePayloadSchema,
    res: BrowserTabSchema,
  }),
  "shell.tab_switch": chan({ req: TabIdPayloadSchema, res: EmptySchema }),
  "shell.tab_close": chan({ req: TabIdPayloadSchema, res: EmptySchema }),
  "shell.tab_open_singleton": chan({
    req: ShellTabOpenSingletonPayloadSchema,
    res: ShellTabOpenSingletonResponseSchema,
  }),
  "shell.tab_new_kind": chan({
    req: ShellNewTabKindPayloadSchema,
    res: ShellNewTabKindResponseSchema,
  }),
  // Tab identity: returns the calling tab's id + arbitrary metadata.
  "shell.this_tab_id": chan({
    req: EmptySchema,
    res: z.object({
      tabId: z.string(),
      meta: z.record(z.string()),
    }),
  }),
  // Renderer-push: set one metadata key on the calling tab.
  "shell.tab_set_meta": chan({
    req: z.object({ key: z.string(), value: z.string() }),
    res: EmptySchema,
  }),
  // refine-ui-theme-layout: open Settings as a top-of-window popover
  "shell.settings_popover_open": chan({
    req: ShellSettingsPopoverOpenPayloadSchema,
    res: ShellSettingsPopoverOpenResponseSchema,
  }),

  // refine-ui-theme-layout: theme persistence + system follow
  "theme.get": chan({ req: EmptySchema, res: ThemeGetResponseSchema }),
  "theme.set": chan({ req: ThemeSetPayloadSchema, res: EmptySchema }),

  // ── arc-style-tab-cards: per-tab toolbar + chrome theme push ──────
  "tab.set_toolbar_state": chan({
    req: TabSetToolbarStatePayloadSchema,
    res: EmptySchema,
  }),
  "tab.set_chrome_theme": chan({
    req: TabSetChromeThemePayloadSchema,
    res: EmptySchema,
  }),

  // ── terminal ───────────────────────────────────────────────────────
  "terminal.list": chan({ req: EmptySchema, res: TerminalListResponseSchema }),
  "terminal.new": chan({ req: EmptySchema, res: TerminalRowSchema }),
  "terminal.switch": chan({ req: TerminalIdPayloadSchema, res: EmptySchema }),
  "terminal.close": chan({ req: TerminalIdPayloadSchema, res: EmptySchema }),
  "terminal.restart": chan({ req: EmptySchema, res: EmptySchema }),
  "terminal.blocks_load": chan({
    req: TerminalBlocksLoadPayloadSchema,
    res: z.array(TerminalBlockSchema),
  }),
  "terminal.block_save": chan({
    req: TerminalBlockSavePayloadSchema,
    res: EmptySchema,
  }),

  // ── agent ──────────────────────────────────────────────────────────
  "agent.task_from_command": chan({
    req: AgentTaskFromCommandPayloadSchema,
    res: EmptySchema,
  }),
  "tool.exec": chan({
    req: ToolExecPayloadSchema,
    res: ToolExecResultSchema,
  }),
  "permission.respond": chan({
    req: PermissionRespondPayloadSchema,
    res: EmptySchema,
  }),
  "llm.config.get": chan({ req: EmptySchema, res: LlmConfigSchema }),
  "llm.config.set": chan({
    req: LlmConfigSetPayloadSchema,
    res: EmptySchema,
  }),

  // ── space ──────────────────────────────────────────────────────────
  "space.list": chan({ req: EmptySchema, res: z.array(SpaceSchema) }),
  "space.create": chan({
    req: z.object({
      name: z.string().optional(),
      root_path: z.string(),
      profile_id: z.string().optional(),
    }),
    res: SpaceSchema,
  }),
  "space.switch": chan({
    req: z.object({ space_id: z.string() }),
    res: EmptySchema,
  }),
  "space.delete": chan({
    req: z.object({ space_id: z.string() }),
    res: EmptySchema,
  }),

  // ── agent-event-bus ────────────────────────────────────────────────
  "events.list": chan({ req: EventsListReq, res: EventsListRes }),
  "events.subscribe": chan({
    req: EventsSubscribeReq,
    res: EventsSubscribeRes,
  }),
  "events.append": chan({ req: EventsAppendReq, res: EventsAppendRes }),

  // ── inbox ──────────────────────────────────────────────────────────
  "inbox.list": chan({ req: InboxListReq, res: InboxListRes }),
  "inbox.read": chan({ req: InboxStateChangeReq, res: EmptySchema }),
  "inbox.unread": chan({ req: InboxStateChangeReq, res: EmptySchema }),
  "inbox.snooze": chan({ req: InboxSnoozeReq, res: EmptySchema }),

  // ── notifications (renderer prefs) ─────────────────────────────────
  "notifications.get_prefs": chan({
    req: EmptySchema,
    res: z.object({
      enabled: z.array(z.string()),
    }),
  }),
  "notifications.set_kind_pref": chan({
    req: z.object({ kind: z.string(), enabled: z.boolean() }),
    res: EmptySchema,
  }),

  // ── documents (DocumentStore-backed) ──────────────────────────────
  // Read / write / list of `<workspace>/.cronymax/flows/<flow>/docs/*.md`.
  // The workbench panel uses `document.read` + `document.submit`; the
  // channel panel and inbox use `document.list` + `document.subscribe`.
  "document.list": chan({
    req: z.object({ flow: z.string() }),
    res: z.object({
      docs: z.array(
        z.object({
          name: z.string(),
          latest_revision: z.number().int().nonnegative(),
        }),
      ),
    }),
  }),
  "document.read": chan({
    req: z.object({
      flow: z.string(),
      name: z.string(),
      // Optional: read a specific historical revision. When omitted the
      // current revision is returned.
      revision: z.union([z.number().int().positive(), z.string()]).optional(),
    }),
    res: z.object({
      revision: z.number().int().nonnegative(),
      content: z.string(),
    }),
  }),
  "document.submit": chan({
    req: z.object({
      flow: z.string(),
      name: z.string(),
      content: z.string(),
    }),
    res: z.object({
      ok: z.literal(true),
      revision: z.number().int().positive(),
      sha: z.string(),
    }),
  }),
  "document.subscribe": chan({
    req: z.object({ flow: z.string() }),
    res: z.object({ ok: z.literal(true), event: z.string() }),
  }),

  // ── reviews (legacy ReviewStore-backed) ────────────────────────────
  "review.list": chan({
    req: z.object({ flow: z.string(), run_id: z.string() }),
    res: z.object({
      docs: z.record(
        z.string(),
        z.object({
          current_revision: z.number().int().nonnegative(),
          status: z.string(),
          round_count: z.number().int().nonnegative().optional(),
          review_exhausted: z.boolean().optional(),
          revisions: z
            .array(
              z.object({
                rev: z.number().int().nonnegative(),
                submitted_at: z.string().optional(),
                submitted_by: z.string().optional(),
                sha: z.string().optional(),
              }),
            )
            .optional()
            .default([]),
          comments: z
            .array(
              z.object({
                id: z.string(),
                author: z.string(),
                kind: z.string(),
                anchor: z.string(),
                body: z.string(),
                block_id: z.string().optional(),
                suggestion: z.string().optional(),
                legacy_anchor: z.string().optional(),
                resolved_in_rev: z.number().int().optional(),
                created_at_ms: z.number().optional(),
              }),
            )
            .optional()
            .default([]),
        }),
      ),
    }),
  }),
  "review.approve": chan({
    req: z.object({
      flow: z.string().optional(),
      run_id: z.string().optional(),
      name: z.string().optional(),
      review_id: z.string().optional(),
      body: z.string().optional(),
    }),
    res: z.unknown(),
  }),
  "review.request_changes": chan({
    req: z.object({
      flow: z.string().optional(),
      run_id: z.string().optional(),
      name: z.string().optional(),
      review_id: z.string().optional(),
      body: z.string().optional(),
    }),
    res: z.unknown(),
  }),
  "review.comment": chan({
    req: z.object({
      flow: z.string(),
      run_id: z.string(),
      name: z.string(),
      body: z.string(),
      // Block-anchored extension (`change: document-wysiwyg`). When
      // present the native side stores the comment with `block_id` set
      // and writes the anchor as `block=<uuid>` instead of `rev=N`.
      block_id: z.string().optional(),
      // Optional Markdown body the user can later "Accept" via
      // `document.suggestion.apply` to materialise a new revision.
      suggestion: z.string().optional(),
    }),
    res: z.unknown(),
  }),

  // ── document suggestion apply (block-anchored review → revision) ──
  // Materialises the `suggestion` body of a block-anchored comment into
  // a new document revision. The native side replaces the block whose
  // `<!-- block: <uuid> -->` marker matches `block_id`, writes the new
  // revision via DocumentStore, and marks the comment resolved.
  "document.suggestion.apply": chan({
    req: z.object({
      flow: z.string(),
      run_id: z.string(),
      name: z.string(),
      comment_id: z.string(),
    }),
    res: z.object({
      ok: z.literal(true),
      new_revision: z.number().int().positive(),
      sha: z.string(),
    }),
  }),

  // ── flow run control ────────────────────────────────────────────────
  // flow.run.cancel is handled via the direct runtime IPC path (flowRun.cancel()).

  // ── workspace custom prompts ───────────────────────────────────────
  // Lists *.prompt.md files from <workspace>/.cronymax/prompts/.
  "workspace.prompts.list": chan({
    req: EmptySchema,
    res: z.object({
      prompts: z.array(z.object({ name: z.string(), content: z.string() })),
    }),
  }),

  // Saves a single prompt file to <workspace>/.cronymax/prompts/<name>.prompt.md.
  "workspace.prompt.save": chan({
    req: z.object({ name: z.string(), content: z.string() }),
    res: z.object({ ok: z.boolean(), error: z.string().optional() }),
  }),

  // ── LLM providers (managed list, persisted in kv_config) ──────────
  // Storage is opaque on the C++ side: the frontend owns the JSON
  // schema for the provider list. `raw` is a JSON array string of
  // {id, name, kind, base_url, api_key, default_model}.
  "llm.providers.get": chan({
    req: EmptySchema,
    res: z.object({
      raw: z.string(),
      active_id: z.string(),
    }),
  }),
  "llm.providers.set": chan({
    req: z.object({
      raw: z.string(),
      active_id: z.string(),
    }),
    res: z.object({ ok: z.boolean() }),
  }),

  // ── LLM provider registry (Rust-backed, typed CRUD + OAuth) ───────────
  // These channels use the new LlmProviderRegistry (Rust) via the runtime
  // bridge. Secrets are stored in the macOS Keychain — never in the JSON.

  /** List all configured providers. */
  "llm.provider.list": chan({
    req: EmptySchema,
    res: z.object({
      providers: z.array(
        z.object({
          id: z.string(),
          kind: z.enum(["openai_compat", "github_copilot", "none"]),
          base_url: z.string(),
          model_override: z.string().nullable().optional(),
          /** "authenticated" | "configured" | "unconfigured" */
          status: z.enum(["authenticated", "configured", "unconfigured"]),
        }),
      ),
      default_provider: z.string().nullable(),
    }),
  }),

  /** Add or replace a provider entry. API key stored in Keychain. */
  "llm.provider.add": chan({
    req: z.object({
      id: z.string(),
      kind: z.enum(["openai_compat", "github_copilot", "none"]),
      base_url: z.string(),
      model_override: z.string().nullable().optional(),
      api_key: z.string().optional(),
      set_as_default: z.boolean().optional(),
    }),
    res: z.object({ ok: z.boolean(), error: z.string().optional() }),
  }),

  /** Remove a provider by id. Also deletes its Keychain item. */
  "llm.provider.remove": chan({
    req: z.object({ id: z.string() }),
    res: z.object({ ok: z.boolean(), error: z.string().optional() }),
  }),

  /**
   * Start a GitHub Copilot device flow.
   * Returns user_code + verification_uri for display; C++ begins polling
   * in the background and emits `llm.provider.auth_status` events.
   */
  "llm.provider.auth_start": chan({
    req: z.object({ provider_id: z.string() }),
    res: z.object({
      ok: z.boolean(),
      user_code: z.string().optional(),
      verification_uri: z.string().optional(),
      error: z.string().optional(),
    }),
  }),

  /**
   * Server-push event emitted by C++ as a device flow progresses.
   * phase: "polling" | "success" | "error" | "expired"
   */
  "llm.provider.auth_status": chan({
    req: EmptySchema,
    res: z.object({
      provider_id: z.string(),
      phase: z.enum(["polling", "success", "error", "expired"]),
      error: z.string().optional(),
    }),
  }),

  "doc_type.list": chan({
    req: EmptySchema,
    res: z.object({
      doc_types: z.array(
        z.object({
          name: z.string(),
          display_name: z.string(),
          user_defined: z.boolean(),
        }),
      ),
    }),
  }), // doc_type.load/save/delete handled via direct runtime IPC (docType.*())

  // ── named sandbox profiles (stored in ~/.cronymax/profiles/) ─────────
  "profiles.list": chan({
    req: EmptySchema,
    res: z.array(
      z.object({
        id: z.string(),
        name: z.string(),
        memory_id: z.string(),
        allow_network: z.boolean(),
        extra_read_paths: z.array(z.string()),
        extra_write_paths: z.array(z.string()),
        extra_deny_paths: z.array(z.string()),
      }),
    ),
  }),
  "profiles.create": chan({
    req: z.object({
      name: z.string(),
      memory_id: z.string().optional(),
      allow_network: z.boolean(),
      extra_read_paths: z.array(z.string()),
      extra_write_paths: z.array(z.string()),
      extra_deny_paths: z.array(z.string()),
    }),
    res: z.object({ ok: z.boolean() }),
  }),
  "profiles.update": chan({
    req: z.object({
      id: z.string(),
      name: z.string(),
      memory_id: z.string().optional(),
      allow_network: z.boolean(),
      extra_read_paths: z.array(z.string()),
      extra_write_paths: z.array(z.string()),
      extra_deny_paths: z.array(z.string()),
    }),
    res: z.object({ ok: z.boolean() }),
  }),
  "profiles.delete": chan({
    req: z.object({ id: z.string() }),
    res: z.object({ ok: z.boolean() }),
  }),
  "profiles.check_paths": chan({
    req: z.object({ paths: z.array(z.string()) }),
    res: z.object({ missing: z.array(z.string()) }),
  }),

  // ── Activity panel ──────────────────────────────────────────────────────
  "activity.snapshot": chan({
    req: z.object({}).optional(),
    res: z.object({
      runs: z.array(z.record(z.unknown())),
      pending_reviews: z.array(z.record(z.unknown())),
    }),
  }),

  // ── flow read — handled via direct runtime IPC (flow.list(), flow.load()) ─
} as const;

/** ── inbound (broadcast) event payloads ──────────────────────────── */
export const Events = {
  "terminal.created": TerminalRowSchema,
  "terminal.removed": TerminalIdPayloadSchema,
  "terminal.switched": TerminalIdPayloadSchema,
  "terminal.exit": TerminalExitPayloadSchema,
  "terminal.restart_requested": EmptySchema,
  "popover.url_changed": z.object({ url: z.string() }),
  "popover_chrome.url_changed": z.object({ url: z.string() }),
  "shell.tabs_changed": z.array(BrowserTabSchema),
  "shell.tab_created": TabCreatedSchema,
  "shell.tab_closed": TabClosedSchema,
  "shell.tab_title_changed": TabTitleChangedSchema,
  "shell.tab_url_changed": TabUrlChangedSchema,
  "shell.active_tab_changed": ActiveTabChangedSchema,
  // arc-style-tab-cards: full tab snapshot (string-id world) and activation
  "shell.tabs_list": TabsListSnapshotSchema,
  "shell.tab_activated": TabActivatedEventSchema,
  "shell.space_changed": SpaceChangedSchema,
  "agent.task_from_command": AgentTaskFromCommandPayloadSchema,
  "space.created": SpaceSchema,
  "space.deleted": z.object({ space_id: z.string() }),
  "space.switch_loading": z.object({ loading: z.boolean() }),
  // refine-ui-theme-layout: theme broadcast for all panels
  "theme.changed": ThemeChangedPayloadSchema,

  // ── agent-event-bus broadcast ──────────────────────────────────────
  event: AppEventSchema,
} as const;

/** Channels marked fastPath skip full Zod validation on inbound events
 *  for throughput. The handler still receives the payload but as `unknown`
 *  cast — it must validate itself if needed.
 *
 *  "event" is fast-pathed because it carries both legacy AppEvent payloads
 *  AND runtime-protocol envelopes { tag:"event", subscription, event:{...} }
 *  which do not match AppEventSchema.  Each handler guards its own shape. */
export const FastPathEvents = new Set<keyof typeof Events>(["event"]);

export type ChannelName = keyof typeof Channels;
export type EventName = keyof typeof Events;
export type RequestOf<C extends ChannelName> = z.input<(typeof Channels)[C]["req"]>;
export type ResponseOf<C extends ChannelName> = z.infer<(typeof Channels)[C]["res"]>;
export type EventPayloadOf<E extends EventName> = z.infer<(typeof Events)[E]>;
