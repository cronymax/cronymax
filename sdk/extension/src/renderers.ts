// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · content renderers
//
// `cronymax.content.renderer` L2 EP. v1 supports block-level renderers
// keyed by MIME type. Inline renderers are M1.
//
// At runtime each renderer is hosted in a webview iframe (see window.ts).
// Discovery and lifecycle are platform-driven; the extension simply
// `registerRenderer` for one or more MIME types it can handle.

import type { Disposable, Event } from "./primitives";

export interface RenderRequest {
  /** Stable id of this render instance (one per displayed block). */
  instanceId: string;
  mimeType: string;
  data: Uint8Array;
  /** Optional structured metadata extracted from the source block. */
  metadata?: Record<string, unknown>;
  /** Theme hint for the renderer ("light" | "dark" | "auto"). */
  theme?: "light" | "dark" | "auto";
}

export interface RenderHandle extends Disposable {
  readonly instanceId: string;
  /** Push a metadata or data update (e.g. streaming partial mermaid source). */
  update(patch: Partial<RenderRequest>): Promise<void>;
  readonly onDidChangeTheme: Event<"light" | "dark" | "auto">;
}

export type RenderHandler = (req: RenderRequest) => RenderHandle | Promise<RenderHandle>;

export interface Renderers {
  /**
   * Register a renderer.
   *
   * `id` MUST match a manifest contribution under
   * `contributes["cronymax.content.renderer"]`.
   *
   * The handler is invoked once per visible block. The returned `RenderHandle`
   * controls update / dispose. Multiple visible blocks → multiple handles.
   */
  registerRenderer(id: string, handler: RenderHandler): Disposable;
}
