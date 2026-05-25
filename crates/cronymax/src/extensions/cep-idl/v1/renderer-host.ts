// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · renderer host (iframe-side)
//
// This file is the SDK surface for code running INSIDE a renderer iframe
// (`cronymax-webview://<extId>/<entry>?surface=renderer&id=<...>`). It is
// NOT part of the Node-side `cronymax` namespace (see index.ts) — renderer-
// only extensions may exist without `manifest.main` at all.
//
// The platform injects `acquireCronymaxRendererApi()` as a global in the
// iframe's V8 context (mirror of the `acquireCronymaxApi()` injection used
// by `createWebviewPanel`; the two are mutually exclusive on a given
// iframe — the `?surface=` query selects which one is bound).
//
// Lifecycle:
//
//   1. Iframe loads. The first script call to `acquireCronymaxRendererApi()`
//      returns an `RendererActivationApi`. A second call throws (acquired-
//      once semantics).
//   2. The iframe calls `api.activate(fn)` exactly once with its
//      `RendererActivate` function.
//   3. The platform invokes the activate fn with a `RendererContext`; the
//      activate fn returns (or resolves to) the `RendererApi` impl.
//   4. For each visible block matching this `(extension, rendererId)`, the
//      platform calls `renderItem(element, request, token)`. For streaming
//      / subsequent updates the platform calls `updateItem` (if provided);
//      otherwise it falls back to calling `renderItem` again on the same
//      element.
//   5. When a block scrolls off / is replaced, `disposeItem(instanceId)`
//      fires (if provided).
//
// The iframe is fully responsible for its own DOM inside the slot the
// platform gives it (`element`). The platform DOES NOT sanitize DOM written
// by the renderer — the iframe is an independent origin (host = extension
// id) with a CSP scoped to the extension, so XSS is contained per-extension
// by the browser's origin model.

import type { CancellationToken, Disposable, Event } from "./primitives";
import type { RenderRequest } from "./renderers";

// ─── activation handle ─────────────────────────────────────────────────────

/**
 * The object returned by `acquireCronymaxRendererApi()`. The renderer iframe
 * calls `activate(fn)` exactly once to register its activate fn.
 */
export interface RendererActivationApi {
  /**
   * Register the renderer's activate function. Calling more than once on
   * the same iframe throws an `Error` (acquired-once mirror of the webview
   * `acquireCronymaxApi()` semantics).
   */
  activate(fn: RendererActivate): void;
}

/**
 * The activate function the renderer iframe defines. Called once per iframe
 * load; returns the per-instance `RendererApi` impl.
 */
export type RendererActivate = (ctx: RendererContext) => RendererApi | Promise<RendererApi>;

// ─── render context ────────────────────────────────────────────────────────

/**
 * Context handed to `RendererActivate`. Backed by V8 handlers wired to the
 * postMessage bridge between the iframe and its parent React surface (and,
 * optionally, the extension's Node host if it has one).
 */
export interface RendererContext {
  /** The renderer id this iframe was loaded for. */
  readonly rendererId: string;
  /** The owning extension id. */
  readonly extensionId: string;

  /** Current theme (v1 is light/dark only; high-contrast is M1). */
  readonly theme: RendererTheme;
  /** Fires when the host theme changes. */
  readonly onDidChangeTheme: Event<RendererTheme>;

  /**
   * Report the iframe's content height in CSS pixels. Required because the
   * parent React surface CANNOT measure cross-origin iframe content — every
   * renderer MUST call this at least once (typically after each render) or
   * its iframe will be clipped to a default height.
   *
   * Renderers that change size should attach a `ResizeObserver` to their
   * root element and forward updates.
   */
  setHeight(px: number): void;

  /**
   * Talk back to the extension's Node host. ONLY available if the extension
   * has `manifest.main` declared; otherwise `undefined`. Use for things the
   * Node side needs to coordinate (e.g. fetching a remote diagram source
   * before render). Resolves `true` once Node ACKs; `false` if the
   * extension is deactivating.
   */
  postMessage?(payload: unknown): Promise<boolean>;
  /** Mirror of postMessage availability. */
  readonly onDidReceiveMessage?: Event<unknown>;
}

export type RendererTheme = "light" | "dark";

// ─── renderer api ──────────────────────────────────────────────────────────

/**
 * The renderer's per-iframe impl, returned by `RendererActivate`. One iframe
 * (one `(extension, rendererId)` pair) handles many block instances; the
 * `element` argument identifies which slot the platform wants painted.
 */
export interface RendererApi {
  /**
   * Render an `item` into the platform-provided `element`. The element is
   * empty when this is first called for a given `instanceId`; the renderer
   * may mutate it freely. The platform owns the element's existence — do
   * NOT cache the reference past this call (use `instanceId` for state).
   *
   * `token.isCancellationRequested` may flip mid-call if the user scrolls
   * the block off-screen or the platform supersedes this request with a
   * newer version. Async renderers MUST honor cancellation.
   */
  renderItem(element: HTMLElement, item: RenderRequest, token: CancellationToken): void | Promise<void>;

  /**
   * Update an existing render in place. Called for streaming partial
   * updates of the same `instanceId`. If omitted, the platform calls
   * `renderItem` again on the same element.
   */
  updateItem?(element: HTMLElement, item: RenderRequest, token: CancellationToken): void | Promise<void>;

  /**
   * Release any per-instance resources (event listeners, timers, etc.).
   * Fired when the block is no longer visible / has been replaced.
   */
  disposeItem?(instanceId: string): void;
}

// ─── global injection ──────────────────────────────────────────────────────

declare global {
  /**
   * Available only inside iframes loaded from `cronymax-webview://` with
   * `?surface=renderer`. Returns `RendererActivationApi`. Calling twice
   * throws.
   */
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  function acquireCronymaxRendererApi(): RendererActivationApi;
}

// Anchor an export so isolatedModules accepts this declare-global file.
export type { Disposable };
