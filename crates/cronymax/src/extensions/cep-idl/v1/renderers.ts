// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · content renderers
//
// `cronymax.content.renderer` L2 EP. Block-level renderers keyed by MIME type.
// Inline renderers are M1.
//
// Architecture: iframe-hosted. Renderer code runs **inside a webview iframe**
// loaded by the platform from `cronymax-webview://<extId>/<entry>?surface=
// renderer&id=<instanceId>`. The iframe entry uses the `acquireCronymax-
// RendererApi()` global (see `renderer-host.ts`) to register its
// `RendererActivate` function; the platform drives the lifecycle by sending
// `RenderRequest`s into the iframe via `iframe.contentWindow.postMessage`
// from the parent React surface.
//
// There is NO Node-side renderer API. A "renderer-only" extension may ship
// without `manifest.main` — only the declarative contribution is required.
//
// This file defines the shared wire shapes (`RenderRequest`). The iframe-side
// SDK lives in `renderer-host.ts`.

// ─── render request ────────────────────────────────────────────────────────

/**
 * One render attempt for a content block. Sent by the platform to the
 * renderer iframe via parent → iframe `postMessage`. The renderer receives
 * it in `RendererApi.renderItem(element, request, token)` (initial mount)
 * or `RendererApi.updateItem?(element, request, token)` (subsequent updates
 * for the same `instanceId`).
 *
 * Streaming semantics:
 *   - `complete: false` means the source block is still being produced
 *     (e.g. mid-LLM-output). The renderer SHOULD try to render but MUST
 *     tolerate parse errors and surface its own fallback (e.g. spinner).
 *   - `complete: true` is the final form. After this no further updates
 *     for the same `instanceId` are sent (unless the user edits the source).
 *   - `version` is a monotonic counter per `instanceId`. Renderers that
 *     await async work MUST drop callbacks whose version is stale (smaller
 *     than the latest seen).
 *
 * v1 is text-first: `content` is a UTF-8 string. Binary renderers will be
 * added in M1 via a sibling field (without breaking this shape).
 */
export interface RenderRequest {
  readonly instanceId: string;
  /**
   * The contributing renderer id, matching `ContentRendererContribution.id`
   * and the registered id seen by `RendererActivate(ctx)` (one iframe per
   * `(extension, rendererId)` pair handles many `instanceId`s).
   */
  readonly rendererId: string;
  readonly mimeType: string;
  /** UTF-8 source of the block. */
  readonly content: string;
  /** False while source is still streaming; true on the final delivery. */
  readonly complete: boolean;
  /** Monotonic per `instanceId`. Discard async results from older versions. */
  readonly version: number;
  readonly metadata?: RenderRequestMetadata;
}

export interface RenderRequestMetadata {
  /** Source language tag for fenced code blocks (e.g. "mermaid"). */
  language?: string;
  /** Where the block came from in the chat / flow. */
  source?: "chat-fence" | "file" | "tool";
  /** Originating chat message id, if applicable. */
  messageId?: string;
}
