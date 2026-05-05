> **Superseded authority model — see `rust-runtime-migration`.** This
> proposal originally placed Skills in a Node.js sidecar that the C++
> host would talk to directly. After `rust-runtime-migration`, the
> Rust runtime (`crates/cronymax`) is the **only** orchestration
> authority. Any future skill execution model — Node sidecar, WASM,
> in-process, or otherwise — MUST be subordinate to Rust runtime
> authority: skills run as runtime-mediated capability adapters or as
> child processes the runtime spawns and supervises, and they
> participate in the runtime's permission, review, and event surfaces.
> Skills MUST NOT have an independent control plane that bypasses
> `cronymax`. Sections below that imply the C++ host directly invokes
> a Node sidecar are kept for historical context but should be read
> through this lens; concrete redesign lands when this change is
> next worked on.

## Why

After `agent-document-orchestration` and `agent-orchestration-ui`, agents have first-class identity and a great UI but a hard-coded skill set baked into the C++ tool registry. That ceiling kills the long-tail value: domain-specific reviewers (security, accessibility, performance), language-specific coders, niche tool integrations. This change makes Skills installable, discoverable, and runnable in a permissioned Node.js sidecar — turning the product into an extensible platform.

## What Changes

- **NEW** Skill package format: a directory bundle with `manifest.json` (name, version, agent_compatibility, declared permissions), `prompts/system.md` + `prompts/examples/`, `tools/index.js` (Node.js entry), `tools/package.json`, `schemas/` (JSON Schemas for tool I/O).
- **NEW** Local skill installation: skills install to `~/.cronymax/skills/<name>/`; per-Space attachment recorded in the Agent's `agent.yaml` (`skills: [<name>@<version>]`).
- **NEW** Node.js sidecar runtime: a single Node child process spawned by the C++ host. Each skill loads in an isolated `vm.Context` with its declared permission set. Tool calls cross the bridge over JSON-RPC over stdio.
- **NEW** Permission model: declared in `manifest.json` (e.g. `fs.read:<path-pattern>`, `fs.write:<path-pattern>`, `net.fetch:<host-pattern>`, `shell.exec`). On first use within a Space, the user is prompted (browser-style permission dialog); decisions persist per Space.
- **NEW** In-app marketplace browser: lists skills from a curated registry (a JSON manifest hosted in a GitHub repo, e.g. `cronymax/skills-registry`); search, install, update, remove. Plus install-from-URL for unlisted skills.
- **NEW** Skill discovery in Agent config: when editing an Agent in the Flow editor, available skills appear in a picker; attaching a skill merges its system-prompt fragment into the Agent's prompt and registers its tools.
- **NEW** Built-in skills bundled with the app: `core-fs`, `core-shell`, `core-git`, `core-web-fetch`, `core-markdown` (plus a `domain-security-reviewer` Critic skill to dogfood the reviewer-agent pattern from `agent-document-orchestration`).
- **MODIFIED** `agent-entity`: `agent.yaml` gains a `skills:` list; `AgentRuntime` resolves tools via the skill runtime instead of (or in addition to) the C++ tool registry.
- **MODIFIED** `multi-agent-orchestration`: tool execution path can route to either the in-process C++ tool registry (built-in) or the Node sidecar (skill-provided). The same JSON tool-call interface for both.
- **NOT IN SCOPE**: hosted marketplace backend, ratings/reviews, payment, code signing/verification, automatic updates, telemetry. Distribution is via a public GitHub repo for v1.

## Capabilities

### New Capabilities

- `agent-skills`: skill package format, manifest schema, local installation, attachment to Agents, prompt/tool merging.
- `skill-runtime`: Node.js sidecar process, `vm.Context` isolation per skill, JSON-RPC bridge, lifecycle (spawn/restart/kill), error recovery.
- `skill-permissions`: permission grammar in manifests, runtime enforcement at the bridge layer, user-facing permission prompts, per-Space persistence.
- `skill-marketplace`: registry.json fetch from configurable Git URL, in-app browser UI, install/update/remove flows, install-from-URL.

### Modified Capabilities

- `agent-entity`: adds `skills:` list to `agent.yaml`; resolution semantics for skill tools and prompt fragments.
- `multi-agent-orchestration`: tool dispatch unifies built-in (C++) and skill-provided (Node) tools behind the same interface.

## Impact

- **NEW C++ module `src/skills/`**: `skill_runtime.{h,cc}` (manages Node sidecar), `skill_loader.{h,cc}` (parses manifest, validates permissions), `skill_bridge.{h,cc}` (JSON-RPC over stdio).
- **NEW `web/skills/` UI**: marketplace browser, permission prompts.
- **NEW Node project under `runtime/skill-host/`**: the sidecar entry point; loads skills into `vm.Context`, enforces permissions, proxies tool calls.
- **New runtime dependency**: Node.js (>=20). Detect at startup; surface friendly install instructions if missing. Bundling Node is deferred (large binary cost).
- **Bridge channels**: `skill.list`, `skill.install`, `skill.uninstall`, `skill.update`, `skill.permission.request/respond`, `marketplace.search`.
- **Filesystem contract**: `~/.cronymax/skills/<name>@<version>/`; per-Space `permissions.json` under `.cronymax/`.
- **Security posture**: permissions are advisory until enforced by the sidecar; this change MUST land the enforcement in the same release as the API. No "trust the manifest" shortcuts.
