# @cronymax/extension

TypeScript SDK for writing Cronymax extensions.

```ts
import { window, commands } from "@cronymax/extension";
import type { ExtensionContext } from "@cronymax/extension";

export async function activate(ctx: ExtensionContext) {
  ctx.subscriptions.push(
    commands.registerCommand("alice.hello", () => {
      window.showInformationMessage("Hello from Alice!");
    }),
  );
}
```

## How it works

When the Cronymax platform spawns your extension, it loads it inside a Node
26 host process (see `bundled/extension-host-bootstrap.js` in the cronymax
repo). The host installs a live `cronymax` namespace on `globalThis`; this
package's runtime entry (`runtime.ts`) reads it and re-exports the surface
so user code can pull values straight from the package name.

The package is type-rich: the entire v1 IDL is mirrored as TypeScript
interfaces. The runtime is a thin shim — the real implementation lives in
the cronymax Rust core and is reached over MessagePack-RPC on inherited
fd 3.

## Status

v1 alpha. The IDL is frozen; the runtime surface area is still growing.

## License

Apache-2.0
