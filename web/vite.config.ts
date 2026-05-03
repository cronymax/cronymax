/// <reference types="vitest" />
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { resolve } from "node:path";

// Vite emits `<script type="module" crossorigin ...>` and
// `<link rel="stylesheet" crossorigin ...>` in production HTML. When the
// resulting bundle is served from `file://` (CEF), Chromium treats the
// crossorigin attribute as opt-in to CORS, and file:// requests have no
// Origin header — so the resource is rejected and the panel renders blank.
// Strip the attribute from emitted HTML so CEF can load assets.
function stripCrossorigin(): Plugin {
  return {
    name: "cronymax-strip-crossorigin",
    enforce: "post",
    transformIndexHtml(html) {
      return html.replace(/\s+crossorigin(?=[\s>])/g, "");
    },
  };
}

// Multi-entry build: every CEF panel has its own flat HTML entry under
// `web/src/panels/`. Vite's `root` is set to `src/panels` so panel URLs collapse
// to a single segment (`/agent.html`, `/agent-graph.html`, …) in both dev
// and production. We disable Vite's own publicDir handling because `root`
// is already pointed at it; otherwise Vite would refuse to start ("publicDir
// must not be inside root").
const srcDir = resolve(__dirname, "src");
const panelsDir = resolve(srcDir, "panels");

const panelEntries = {
  popover: resolve(panelsDir, "popover/index.html"),
  sidebar: resolve(panelsDir, "sidebar/index.html"),
  terminal: resolve(panelsDir, "terminal/index.html"),
  chat: resolve(panelsDir, "chat/index.html"),
  agent: resolve(panelsDir, "agent/index.html"),
  flow: resolve(panelsDir, "chat/index.html"),
  channel: resolve(panelsDir, "channel/index.html"),
  editor: resolve(panelsDir, "editor/index.html"),
  inbox: resolve(panelsDir, "inbox/index.html"),
  workbench: resolve(panelsDir, "workbench/index.html"),
  settings: resolve(panelsDir, "settings/index.html"),
  "popover-chrome": resolve(panelsDir, "popover-chrome/index.html"),
};

export default defineConfig({
  base: "./",
  root: srcDir,
  publicDir: false,
  plugins: [react(), tailwindcss(), stripCrossorigin()],
  resolve: {
    alias: {
      "@": srcDir,
    },
  },
  server: {
    port: 5173,
    strictPort: true,
    cors: true,
    // Allow CEF to load HMR client over file:/// origin.
    headers: {
      "Access-Control-Allow-Origin": "*",
    },
    // `root` is `public/`, but panel HTMLs import `../src/...`.
    // Whitelist the parent web/ dir so Vite's dev server can serve those
    // out-of-root files.
    fs: {
      allow: [resolve(__dirname, "..")],
    },
  },
  build: {
    // Emit into web/dist/ (sibling of public/) so CMake's copy_directory
    // step keeps working unchanged.
    outDir: resolve(__dirname, "dist"),
    emptyOutDir: true,
    rollupOptions: {
      input: panelEntries,
    },
  },
  // Vitest config — `root` above points at public/ for the build, but
  // tests live under web/test/, so override the test root back to web/.
  test: {
    root: __dirname,
  },
});
