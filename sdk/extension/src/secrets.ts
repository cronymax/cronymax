// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · secrets
//
// Per-extension secret store backed by the OS keychain:
//   - macOS  → Security framework / Keychain
//   - Linux  → libsecret / secret-service
//   - Win    → DPAPI / Credential Manager
//
// Keys are scoped by manifest `capabilities.secrets.namespace`. The namespace
// MUST be a prefix of `<publisher>.*`. Writes outside the namespace error
// with `ERR_NAMESPACE`.

import type { Event } from "./primitives";

export interface Secrets {
  get(key: string): Promise<string | undefined>;
  set(key: string, value: string): Promise<void>;
  delete(key: string): Promise<void>;

  /** Fires when another agent in the same OS process touches this key. */
  readonly onDidChange: Event<{ key: string }>;
}

export type SecretsErrorCode = "ERR_NAMESPACE" | "ERR_BACKEND_UNAVAILABLE" | "ERR_USER_CANCELLED";

export interface SecretsError extends Error {
  readonly code: SecretsErrorCode;
}
