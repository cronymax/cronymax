// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · workspace
//
// Workspace abstraction. Filesystem access here is a thin URI wrapper over
// Node `fs/promises`; ACL is enforced by Node 26 `--allow-fs-read/-write`
// flags injected by the platform from `capabilities.fs`.

import type { Disposable, Event, URI } from "./primitives";

export interface Workspace {
  /**
   * All currently-opened workspace roots, in display order. Empty when
   * cronymax is in "no workspace" mode. Multi-root workspaces produce
   * multiple entries. Every entry's `uri` is canonical (symlinks
   * resolved); the platform grants `rw` on each folder automatically —
   * extensions do NOT need to declare `{WORKSPACE}` in their manifest.
   */
  readonly workspaceFolders: readonly WorkspaceFolder[];

  /**
   * Convenience shortcut to `workspaceFolders[0]?.uri`. Undefined when
   * no workspace is open. Prefer `workspaceFolders` for multi-root
   * support.
   */
  readonly rootUri: URI | undefined;

  /** URI-based filesystem facade over `fs/promises`. */
  readonly fs: WorkspaceFileSystem;

  /** Get a typed configuration scope. Empty section returns the root. */
  getConfiguration(section?: string): Configuration;

  /** Fired when any key under any registered config schema changes. */
  readonly onDidChangeConfiguration: Event<ConfigurationChangeEvent>;
}

export interface WorkspaceFolder {
  /** Canonical `file://` URI of the folder. */
  readonly uri: URI;
}

// ─── filesystem ────────────────────────────────────────────────────────────

export interface FileStat {
  type: "file" | "directory" | "symlink";
  /** Size in bytes for regular files; 0 otherwise. */
  size: number;
  /** Unix epoch milliseconds. */
  ctime: number;
  mtime: number;
}

export interface WorkspaceFileSystem {
  readFile(uri: URI): Promise<Uint8Array>;
  writeFile(uri: URI, content: Uint8Array): Promise<void>;
  delete(uri: URI, options?: { recursive?: boolean }): Promise<void>;
  rename(source: URI, target: URI, options?: { overwrite?: boolean }): Promise<void>;
  copy(source: URI, target: URI, options?: { overwrite?: boolean }): Promise<void>;
  createDirectory(uri: URI): Promise<void>;
  readDirectory(uri: URI): Promise<Array<[string, FileStat["type"]]>>;
  stat(uri: URI): Promise<FileStat>;
}

// ─── configuration ─────────────────────────────────────────────────────────

export interface Configuration {
  /** Read a setting by dotted key. Returns `defaultValue` if absent. */
  get<T = unknown>(key: string, defaultValue?: T): T | undefined;
  /** Same as `get` but throws if the key is unknown to the schema. */
  require<T = unknown>(key: string): T;
  /** Whether the key has an explicit user-set value (not just schema default). */
  has(key: string): boolean;
  /** Write a setting back. Persisted in the user's settings store. */
  update(key: string, value: unknown): Promise<void>;
}

export interface ConfigurationChangeEvent {
  /** Whether the given dotted key (or any descendant if `key` is a prefix) changed. */
  affectsConfiguration(key: string): boolean;
}

/** Workspace.fs ACL-error code; matches Node's `ERR_ACCESS_DENIED`. */
export type WorkspaceFsErrorCode = "ERR_ACCESS_DENIED" | "ENOENT" | "EISDIR" | "ENOTDIR" | "EEXIST" | "EACCES";

export interface WorkspaceFsError extends Error {
  readonly code: WorkspaceFsErrorCode;
  readonly uri?: URI;
}

/** Owned by the platform; defines Disposables that release filesystem watchers etc. */
export type WorkspaceDisposable = Disposable;
