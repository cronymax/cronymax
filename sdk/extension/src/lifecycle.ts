// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · lifecycle
//
// Activate/deactivate contract + ExtensionContext shape.

import type { Disposable, URI } from "./primitives";

/**
 * The `activate` export of a Cronymax extension.
 *
 * MUST resolve before the extension is considered "active". Throwing or
 * rejecting rolls the extension back to enabled-but-inactive and the platform
 * will surface the error to the user.
 */
export type ActivateFn = (ctx: ExtensionContext) => void | Promise<void>;

/**
 * The `deactivate` export. Called when:
 *   - the user disables the extension
 *   - cronymax is shutting down (best-effort; not guaranteed)
 *
 * In v1 cronymax does NOT proactively deactivate an extension to reclaim
 * resources — host stays up until cronymax exits.
 */
export type DeactivateFn = () => void | Promise<void>;

export type ExtensionMode = "production" | "development" | "test";

export interface ExtensionContext {
  /** Manifest `id`. */
  readonly extensionId: string;
  /** Manifest. Frozen snapshot at activation time. */
  readonly manifest: import("./manifest").Manifest;
  /**
   * Disposables registered here are disposed on deactivate(). Idiomatic usage:
   *   ctx.subscriptions.push(cronymax.commands.register(...));
   */
  readonly subscriptions: Disposable[];

  /** Path to the extension install directory. */
  readonly extensionUri: URI;
  /** Per-extension private storage (R/W always granted by Node flags). */
  readonly storageUri: URI;
  /** Per-extension global storage (R/W across workspaces). */
  readonly globalStorageUri: URI;
  /** Per-extension log file dir. */
  readonly logUri: URI;

  readonly extensionMode: ExtensionMode;

  /**
   * Exports object — set by the extension during activate to expose its API
   * to other extensions via `cronymax.extensions.getExtension(id).exports`.
   * VS Code-style: no schema, no semver, contract is between the two
   * extensions only.
   */
  exports: unknown;
}
