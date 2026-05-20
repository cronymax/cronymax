// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · extensions
//
// Cross-extension introspection. Mirrors VS Code's `vscode.extensions`.
// Cross-extension contracts are exchanged through `Extension.exports` —
// no schema, no semver, no platform mediation. Two extensions agreeing on
// a shape is between them.

import type { Manifest } from "./manifest";

export interface Extension<TExports = unknown> {
  /** `<publisher>.<name>`. */
  readonly id: string;
  readonly manifest: Manifest;
  readonly isActive: boolean;
  /**
   * Reference to whatever the extension assigned to `ctx.exports` during
   * activation. Unknown if the extension is not active yet — call
   * `activate()` first.
   */
  readonly exports: TExports | undefined;
  /** Activate the extension (no-op if already active). Resolves once `activate()` returns. */
  activate(): Promise<TExports>;
}

export interface ExtensionsNamespace {
  /** Lookup by manifest id. Returns undefined if not installed. */
  getExtension<TExports = unknown>(id: string): Extension<TExports> | undefined;
  /** All installed extensions, regardless of enabled / active state. */
  readonly all: readonly Extension[];
}
