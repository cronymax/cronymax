// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · commands
//
// Named, addressable callable units. Extensions register commands under their
// own publisher namespace; the platform routes `commands.execute(id, ...)` to
// the owning extension and serialises args/return through MessagePack-RPC.

import type { Disposable } from "./primitives";

export type CommandHandler<TArgs extends unknown[] = unknown[], TResult = unknown> = (
  ...args: TArgs
) => TResult | Promise<TResult>;

export interface Commands {
  /**
   * Register a command. `id` MUST be inside the extension's publisher
   * namespace (`<publisher>.<anything>`). Writing to `cronymax.*` is rejected
   * at registration.
   *
   * The returned Disposable removes the registration. Re-registering the same
   * id (after disposal) is allowed.
   */
  register<TArgs extends unknown[] = unknown[], TResult = unknown>(
    id: string,
    handler: CommandHandler<TArgs, TResult>,
  ): Disposable;

  /**
   * Invoke any registered command by id. Cross-extension calls are routed by
   * the platform; the caller must have `extension-points` or a peer
   * `extensionDependencies` claim on the target (platform-defined; M1).
   */
  execute<TResult = unknown>(id: string, ...args: unknown[]): Promise<TResult>;

  /** All currently-registered command ids visible to this extension. */
  list(): Promise<string[]>;
}
