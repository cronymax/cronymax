// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · auth
//
// Built-in OAuth / PKCE / device-flow. Sessions are persisted by the
// platform; extensions never see refresh tokens, only short-lived access
// tokens (re-issued on demand).

import type { Event } from "./primitives";

export interface AuthSession {
  readonly id: string;
  readonly accessToken: string;
  /** Identity-provider scopes granted for this session. */
  readonly scopes: readonly string[];
  /** Opaque, provider-specific account identifier (email, sub, etc.). */
  readonly account: { id: string; label: string };
}

export interface GetSessionOptions {
  /**
   * If true and no session matches, prompt the user to sign in.
   * Default false → return undefined when no cached session.
   */
  createIfNone?: boolean;
  /**
   * If true and a session exists, force a re-auth (e.g. to bump scope).
   * Default false.
   */
  forceNewSession?: boolean;
  /**
   * If true and multiple sessions match, prompt the user to pick.
   * Default false → returns the most recently used.
   */
  clearSessionPreference?: boolean;
}

export interface Auth {
  /**
   * Lookup-or-create flow for an OAuth-style session.
   *
   * `providerId` MUST be present in `capabilities.auth.providers`.
   * Returns undefined only when `createIfNone` is false and no session
   * exists.
   */
  getSession(
    providerId: string,
    scopes: readonly string[],
    options?: GetSessionOptions,
  ): Promise<AuthSession | undefined>;

  /** Force-clear a session (logout). */
  removeSession(providerId: string, sessionId: string): Promise<void>;

  /**
   * Fired when sessions for `providerId` are added / removed / renewed.
   * Includes events triggered by other extensions on the same provider.
   */
  readonly onDidChangeSessions: Event<{ providerId: string }>;
}
