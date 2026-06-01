// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · primitives
//
// Foundational types referenced by every other IDL module.
// FROZEN: any change here is a v1 → v2 break.

/** A uniform resource identifier (RFC 3986). Mirrors VS Code's Uri shape. */
export interface URI {
  readonly scheme: string;
  readonly authority: string;
  readonly path: string;
  readonly query: string;
  readonly fragment: string;
  /** `scheme:[//authority]path[?query][#fragment]` */
  toString(): string;
  /** Filesystem path component (decoded). For `file:` URIs only. */
  readonly fsPath: string;
}

/** Anything that owns a resource which must be released. */
export interface Disposable {
  dispose(): void | Promise<void>;
}

export namespace Disposable {
  export function from(...disposables: Disposable[]): Disposable {
    return {
      dispose() {
        for (const d of disposables) {
          try {
            d.dispose();
          } catch {
            /* swallow; per-disposable safety is caller's job */
          }
        }
      },
    };
  }
}

/** Listener fn shape used by every `Event<T>` source. */
export type Listener<T> = (event: T) => void | Promise<void>;

/** Subscribe-able event source. Returns a Disposable that unsubscribes. */
export type Event<T> = (listener: Listener<T>) => Disposable;

/** Cooperative cancellation. */
export interface CancellationToken {
  readonly isCancellationRequested: boolean;
  readonly onCancellationRequested: Event<void>;
}

/** Mutable cancellation source. Owned by the platform; passed read-only to extensions. */
export interface CancellationTokenSource extends Disposable {
  readonly token: CancellationToken;
  cancel(): void;
}

/** Standard error thrown when an operation is cancelled via a token. */
export interface CancellationError extends Error {
  readonly name: "CancellationError";
}

/** Tri-state success result used for RPC-like APIs (Phase 2+ may replace with discriminated union). */
export type Thenable<T> = PromiseLike<T>;
