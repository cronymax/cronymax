# `cronymax-examples.mermaid-renderer`

Renders [Mermaid](https://mermaid.js.org) diagrams for fenced
``` ```mermaid``` ``` blocks in chat and flow outputs. **Declarative-only** —
the manifest contributes `cronymax.content.renderer`, the iframe entry
runs the user's mermaid source through `mermaid.render`, and no Node-side
`main.js` is required.

## Build

The renderer iframe loads a local `./mermaid.min.js`. We don't commit the
upstream bundle (~3 MB); run this once after install to populate it:

```bash
npm install      # pulls mermaid into node_modules
npm run build    # copies node_modules/mermaid/dist/mermaid.min.js → renderer/
```

## Install into cronymax

1. Copy the directory to `~/.cronymax/extensions/cronymax-examples.mermaid-renderer/`.
2. Restart cronymax.
3. Any chat message that contains a fenced block like

   ```text
   ```mermaid
   graph TD
     A --> B
   ```
   ```

   will mount an `<iframe>` that renders the diagram inline.

## How it works (P6.5 design)

- **No Node host.** The manifest omits `main`; the cronymax runtime
  recognises that and skips Node spawn entirely (`ExtensionRuntime::
  activate` declarative-only path). All renderer code runs in the iframe.
- **Iframe scheme.** The chat panel mounts
  `cronymax-webview://cronymax-examples.mermaid-renderer/renderer/index.html?surface=renderer&id=<uuid>`.
  The cronymax-webview scheme handler resolves files relative to the
  extension's install dir under a strict per-origin CSP.
- **Render dispatch.** The parent React surface posts
  `cronymax:renderer:render` and `cronymax:renderer:update` messages with
  the mermaid source; the renderer SDK shim (`acquireCronymaxRendererApi`)
  calls the extension's `renderItem` / `updateItem`.
- **Height reporting.** The iframe is cross-origin and the parent can't
  measure its content. After every render, the extension calls
  `ctx.setHeight(px)` which goes through the renderer V8 binding →
  C++ IPC → Rust `forward_renderer_height` → Authority topic
  `extensions/renderer`, and the parent `<ExtensionContentBlock>`
  component resizes the embedding `<iframe>`.

## Caveats / known limits (v1 alpha)

- **No external network.** The iframe CSP defaults to `connect-src 'self'`,
  so the extension can't fetch from external CDNs. Bundle mermaid locally
  via `npm run build`. If you need other libs to fetch remote diagrams,
  declare allowed hosts in the manifest's `csp.connect_src`.
- **No streaming preview.** The chat panel only mounts the renderer once
  the fence is closed (P6.5 IDL D12); partial mermaid source is shown as
  a regular streaming code block until the closing `` ``` `` arrives.
- **Theme** is a one-shot at iframe load; `onDidChangeTheme` is wired in
  the SDK but the chat surface doesn't push theme updates yet.
