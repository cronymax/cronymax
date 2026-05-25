// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · manifest
//
// `cronymax-extension.json` schema. Mirrored 1:1 by the Rust serde struct in
// `crates/cronymax/src/extensions/manifest.rs`.
//
// FROZEN at v1. Add fields with sensible defaults; never remove or repurpose.

// ─── identity ──────────────────────────────────────────────────────────────

export interface Manifest {
  /** `<publisher>.<name>`. The `<publisher>` segment MUST equal `publisher`. */
  id: string;
  /** Display name. */
  name: string;
  /** SemVer (https://semver.org). */
  version: string;
  /** Publisher slug, lower-case, kebab/dot-free. */
  publisher: string;
  engines: { cronymax: string };
  /**
   * Entry point relative to extension root. CJS in v1; ESM in M1.
   *
   * Optional: extensions that ship ONLY declarative contributions (content
   * renderers without a Node-side coordinator, pure UI sidebar views, etc.)
   * may omit `main` entirely. The platform skips spawning a Node host for
   * such extensions and only ingests their manifest contributions.
   */
  main?: string;

  description?: string;
  icon?: string;
  repository?: string;
  license?: string;
  keywords?: string[];

  /**
   * Triggers that fire `activate(ctx)`. Empty array = never auto-activates;
   * still loadable via `cronymax.extensions.getExtension(id).activate()`.
   *
   * Recognized prefixes (v1):
   *   - `onStartup`
   *   - `onCommand:<commandId>`
   *   - `onAgentProvider:<providerId>`
   *   - `onView:<viewId>`
   *   - `*`  (warned, discouraged; eager activate)
   */
  activationEvents: string[];

  /** L2 EP contributions. Each key must be declared in `capabilities.extension-points`. */
  contributes?: Contributes;

  /** Capability declarations consumed by `build_node_flags` and the install-time consent UI. */
  capabilities?: Capabilities;

  /** IDs of other extensions that MUST activate first. */
  extensionDependencies?: string[];
}

// ─── capabilities ──────────────────────────────────────────────────────────

export interface Capabilities {
  /**
   * Real-fs access. Each entry declares one path scope; the platform expands
   * variables (see `FsPath`) and emits matching `--allow-fs-read` /
   * `--allow-fs-write` flags.
   *
   * The extension's own install dir and per-extension storage dirs are
   * granted by the platform unconditionally and need NOT appear here.
   */
  fs?: readonly FsCapability[];
  /**
   * Real-network access. In v1 this is informational only — the manifest
   * lists hosts the extension intends to reach so the install-time consent
   * UI can surface them, but Node 26's `--allow-net` doesn't yet support
   * per-host filtering. The flag is emitted as a boolean if `fs` is
   * non-empty.
   */
  network?: NetworkCapability;
  /** `child_process` access. boolean → `--allow-child-process`. */
  process?: boolean;
  /** `worker_threads` access. boolean → `--allow-worker`. */
  workers?: boolean;
  /** Native (`.node`) addons. boolean → `--allow-addons`. */
  // eslint-disable-next-line @typescript-eslint/naming-convention
  native_addons?: boolean;

  /** Secret-store access. Namespace must match the extension publisher prefix. */
  secrets?: { namespace: string };

  /** Platform event topic whitelist. */
  "events.subscribe"?: string[];
  /** Platform event topic emit whitelist (must be inside publisher namespace). */
  "events.emit"?: string[];

  /** UI surfaces this extension may target. */
  "ui-slots"?: UiSlot[];

  /** Every key the manifest contributes to MUST be listed here. */
  "extension-points"?: ExtensionPointId[];

  /** Authentication provider IDs this extension may consume. */
  "auth.providers"?: string[];
}

/**
 * One filesystem grant.
 *
 * `path` MUST use one of the platform variables below (no absolute hard-coded
 * paths in v1; the platform rejects them at install-time validation):
 *
 *   {WORKSPACE}            ← current cronymax workspace root
 *   {HOME}/<subpath>       ← user home; bare {HOME} not allowed, must have subpath
 *   {EXT_DIR}              ← extension install dir; always read-granted by the platform anyway
 *   {EXT_STORAGE}          ← per-extension private storage; always rw-granted
 *   {EXT_GLOBAL_STORAGE}   ← per-extension cross-workspace storage; always rw-granted
 *   {TMP}/<subpath>        ← OS temp dir (os.tmpdir())
 *   {CRONYMAX_CONFIG}/<subpath>
 *
 * The platform expands the variable at spawn time, canonicalises the result
 * (resolves symlinks once on macOS), and emits one or two `--allow-fs-*`
 * flags so the extension can use either symlink or canonical forms.
 *
 * Path traversal (`..` to escape the variable's root) is rejected.
 */
export interface FsCapability {
  path: string;
  mode: "r" | "rw";
}

export interface NetworkCapability {
  /**
   * Hosts the extension intends to reach. v1 informational only (Node 26's
   * `--allow-net` is boolean; per-host filtering planned for Node 27+).
   * Surfaced to the user in the install-time consent dialog.
   */
  allow: readonly string[];
}

export type UiSlot = "sidebar" | "settings" | "activitybar" | "statusbar";

// ─── contributions ─────────────────────────────────────────────────────────

export interface Contributes {
  "cronymax.command"?: CommandContribution[];
  "cronymax.config.schema"?: ConfigSchemaContribution;
  "cronymax.config.page"?: ConfigPageContribution[];
  "cronymax.agents.provider"?: AgentProviderContribution[];
  "cronymax.content.renderer"?: ContentRendererContribution[];
  "cronymax.ui.sidebar.view"?: SidebarViewContribution[];
}

export type ExtensionPointId = keyof Contributes;

export interface CommandContribution {
  id: string;
  title: string;
  category?: string;
  icon?: string;
}

/** A JSON Schema fragment (draft 7). Settings UI will auto-render. */
export interface ConfigSchemaContribution {
  title: string;
  properties: Record<string, JsonSchema>;
}

export interface ConfigPageContribution {
  id: string;
  title: string;
  /** Path (relative to extension root) to the entry HTML rendered in a webview. */
  entry: string;
}

export interface AgentProviderContribution {
  id: string;
  label: string;
  icon?: string;
  description?: string;
  supportsModels?: boolean;
  supportsModes?: boolean;
  supportsMcp?: boolean;
}

export interface ContentRendererContribution {
  id: string;
  /**
   * Media types this renderer handles. e.g. "text/vnd.mermaid", "application/json".
   * Renderer scope is `block` in v1; inline is M1.
   */
  mimeTypes: string[];
  /** Renderer HTML entry, loaded into an iframe by the platform. */
  entry: string;
  /**
   * Renderer-iframe CSP overrides. The default CSP applied to the
   * `cronymax-webview://` scheme is
   *   `default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline';
   *    img-src cronymax-webview: data:; connect-src 'self'; font-src 'self' data:`
   * which lets the renderer fetch resources from its own extension dir but
   * NOT the outside network. To allow the renderer to fetch from external
   * hosts (e.g. a remote diagram CDN), declare them here — these hosts are
   * merged into the iframe's `connect-src` directive.
   *
   * This is INDEPENDENT of the extension's Node-side `capabilities.network`:
   * Node fetch and iframe fetch are separate origins, and each must be
   * authorised in its own dimension.
   */
  csp?: RendererCsp;
}

export interface RendererCsp {
  /** Hosts merged into the iframe's `connect-src` CSP directive. */
  connect_src?: string[];
}

export interface SidebarViewContribution {
  id: string;
  title: string;
  icon?: string;
  entry: string;
}

// ─── JSON schema (subset) ──────────────────────────────────────────────────

export interface JsonSchema {
  type?: "string" | "number" | "integer" | "boolean" | "object" | "array" | "null";
  description?: string;
  default?: unknown;
  enum?: unknown[];
  items?: JsonSchema;
  properties?: Record<string, JsonSchema>;
  required?: string[];
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  pattern?: string;
  format?: string;
}
