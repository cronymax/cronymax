# `@cronymax/cep-idl-v1` — Cronymax Extension Platform IDL, v1 (FROZEN)

This directory is the **single source of truth** for the v1 extension-platform
contract. Every Rust runtime type and every public field of the
`@cronymax/extension` SDK is derived from the `.ts` files here.

## What's frozen

Once v1 ships, the following are append-only:

- the shape of `Cronymax` in `index.ts` (the public namespace)
- every `interface` / `type` exported from this directory
- `AgentEvent`'s discriminant `kind` values
- the `Capabilities` keys

Breaking changes require a sibling `v2/` directory.

## What can grow

- New optional fields on existing interfaces (with default fallbacks at the
  consumer side)
- New `AgentEvent` variants, **iff** older clients can ignore them safely
- New platform-event topics under `cronymax.*`
- New L2 extension-point types under `Contributes`
- New SDK namespaces alongside the existing ones in `Cronymax`

## Verifying

```bash
cd crates/cronymax/src/extensions/cep-idl/v1
npm install
npm run check
```

A clean exit is the freeze gate. CI should run this on every PR that touches
this directory.

## File map

| File | Contents |
|---|---|
| `primitives.ts` | URI / Disposable / Event / CancellationToken |
| `manifest.ts`   | `cronymax-extension.json` schema |
| `lifecycle.ts`  | activate/deactivate + ExtensionContext |
| `commands.ts`   | command register/execute |
| `events.ts`     | pub/sub + 8 platform topics |
| `workspace.ts`  | rootUri + fs facade + configuration |
| `window.ts`     | messages, input boxes, webview panels |
| `secrets.ts`    | OS keychain wrapper |
| `auth.ts`       | OAuth/PKCE/device-flow sessions |
| `extensions.ts` | cross-extension getExtension / exports |
| `agents.ts`     | **AgentProvider / AgentSession / AgentEvent** |
| `renderers.ts`  | content renderer registration |
| `index.ts`      | the `Cronymax` namespace + barrel re-exports |
