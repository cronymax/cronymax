#!/usr/bin/env node
// Copy the standalone mermaid.min.js bundle from node_modules into
// renderer/ so the iframe can <script src="./mermaid.min.js"> load it
// without needing network access (the extension declares no CSP
// `connect_src` overrides — see cronymax-extension.json).
//
// We don't check the ~3 MB file into git; this script populates it
// after `npm install` (run via the `build` package.json script).

"use strict";
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const candidates = [
  path.join(root, "node_modules", "mermaid", "dist", "mermaid.min.js"),
  // Workspace-root install via npm/pnpm/yarn.
  path.join(root, "..", "..", "node_modules", "mermaid", "dist", "mermaid.min.js"),
];

// Bun hoists into a versioned `.bun/<pkg>@<ver>/node_modules/...` layout.
// Resolve any installed version by globbing the `.bun` directory.
function bunCandidates() {
  const bunRoot = path.join(root, "..", "..", "node_modules", ".bun");
  if (!fs.existsSync(bunRoot)) return [];
  return fs
    .readdirSync(bunRoot)
    .filter((d) => d.startsWith("mermaid@"))
    .map((d) =>
      path.join(bunRoot, d, "node_modules", "mermaid", "dist", "mermaid.min.js"),
    )
    .filter((p) => fs.existsSync(p));
}

const src = [...candidates, ...bunCandidates()].find((p) => fs.existsSync(p));
if (!src) {
  console.error(
    "[mermaid-renderer] mermaid.min.js not found in any of:\n" +
      candidates.map((p) => "  " + p).join("\n") +
      "\n\nRun `npm install` (or `bun install`) in this directory first.",
  );
  process.exit(1);
}

const dst = path.join(root, "renderer", "mermaid.min.js");
fs.mkdirSync(path.dirname(dst), { recursive: true });
fs.copyFileSync(src, dst);
console.log("[mermaid-renderer] copied", src, "→", dst);
