// SPDX-License-Identifier: Apache-2.0
//
// `@cronymax/extension` runtime entry.
//
// The extension host (`bundled/extension-host-bootstrap.js`) wires a Node
// 26 child process to the Rust platform via inherited fd 3 + MessagePack-
// RPC and installs the live `cronymax` namespace on `globalThis`. This
// module retrieves that global at import time so user code can write the
// canonical `import * as cronymax from "@cronymax/extension"` form
// without any extra setup.
//
// **Outside an extension host process** (e.g. running unit tests against
// extension code in plain Node), import time throws. Tests that exercise
// extension business logic should mock the cronymax surface explicitly
// instead of relying on this module.

import type { Cronymax } from "./index";

interface GlobalWithCronymax {
  cronymax?: Cronymax;
}

const g = globalThis as unknown as GlobalWithCronymax;

if (g.cronymax === undefined) {
  throw new Error(
    "@cronymax/extension: the cronymax runtime is not installed on this " +
      "globalThis. This package can only be imported from inside a cronymax " +
      "extension host process (started via `extension-host-bootstrap.js`).",
  );
}

/** The live cronymax runtime, injected by the extension host. */
export const cronymax: Cronymax = g.cronymax;

/** Convenience flat re-exports so user code can `import { window } from ...`. */
export const env = cronymax.env;
export const commands = cronymax.commands;
export const events = cronymax.events;
export const workspace = cronymax.workspace;
export const window = cronymax.window;
export const secrets = cronymax.secrets;
export const auth = cronymax.auth;
export const extensions = cronymax.extensions;
export const agents = cronymax.agents;
export const renderers = cronymax.renderers;
