// Content-renderer registry for the chat panel.
//
// Loads `cronymax.content.renderer` contributions from the runtime once per
// session (refetched on `runtime.reconnected`) and exposes a `lookupByLang`
// helper used by the fenced-code-block dispatcher in `ContentStreamView`.
//
// Live updates (extension activate / deactivate mid-session) are NOT wired
// in v1 alpha — fenced blocks against a renderer that wasn't loaded at
// chat-panel mount time fall back to the default code block. P10 hardening.

import { useEffect, useRef, useState } from "react";
import { browser, runtime } from "../../shells/bridge";
import { type ContributionDescriptor, ContributionKind, contributionRegistry } from "../../shells/runtime";

export interface ExtensionRenderer {
  /** Owning extension id (`publisher.name`). */
  extId: string;
  /** Renderer id, locally unique within the extension. */
  rendererId: string;
  /** Entry path declared in manifest (e.g. `./renderer/index.html`). */
  entry: string;
  /** MIME types the renderer claims, as declared in the manifest. */
  mimeTypes: string[];
}

/** Metadata payload the Rust side stamps onto content-renderer
 *  descriptors. Mirrors `ContentRendererContribution` from the manifest IDL. */
interface RendererMetadata {
  id?: string;
  mimeTypes?: string[];
  entry?: string;
}

function descriptorToRenderer(desc: ContributionDescriptor): ExtensionRenderer | null {
  if (desc.owner.type !== "extension") return null;
  const md = (desc.metadata as RendererMetadata | undefined) ?? {};
  const entry = typeof md.entry === "string" ? md.entry : "";
  const mimeTypes = Array.isArray(md.mimeTypes) ? md.mimeTypes.filter((m): m is string => typeof m === "string") : [];
  if (!entry || mimeTypes.length === 0) return null;
  return {
    extId: desc.owner.extId,
    rendererId: desc.id,
    entry,
    mimeTypes,
  };
}

/**
 * Resolve a fenced code-block language tag (e.g. "mermaid") to a renderer.
 *
 * Tries, in order:
 *   1. The lang tag as a literal MIME (`mermaid`).
 *   2. `text/vnd.<lang>` — the convention encouraged by the IDL docs.
 *   3. `text/x-<lang>` — the older convention some extensions use.
 *   4. `application/vnd.<lang>`.
 *
 * Returns the first renderer whose `mimeTypes` includes any of these
 * patterns; ties broken in the iteration order of the registry array
 * (which the Rust side sorts deterministically by `(ext_id, renderer_id)`).
 */
export function lookupRendererByLang(registry: ExtensionRenderer[], lang: string): ExtensionRenderer | undefined {
  const probes = [lang, `text/vnd.${lang}`, `text/x-${lang}`, `application/vnd.${lang}`];
  for (const probe of probes) {
    const hit = registry.find((r) => r.mimeTypes.includes(probe));
    if (hit) return hit;
  }
  return undefined;
}

/**
 * Hook that exposes the current renderer registry. Returns an empty array
 * while the initial fetch is in flight or if the runtime is unavailable.
 *
 * Refetches on the `runtime.reconnected` bridge event so a transient
 * runtime restart doesn't strand chat with a stale registry.
 */
export function useExtensionRendererRegistry(): ExtensionRenderer[] {
  const [registry, setRegistry] = useState<ExtensionRenderer[]>([]);
  // Track whether we've kicked off the initial fetch; reconnects always
  // refetch regardless.
  const initialFetched = useRef(false);

  useEffect(() => {
    let cancelled = false;
    const refetch = async () => {
      try {
        const { contributions } = await contributionRegistry.list();
        if (cancelled) return;
        const next = contributions
          .filter((d) => d.kind === ContributionKind.ContentRenderer)
          .map(descriptorToRenderer)
          .filter((r): r is ExtensionRenderer => r !== null);
        setRegistry(next);
      } catch {
        // First load can race the runtime startup banner; the reconnect
        // listener will retry. Silently keep the previous registry.
      }
    };

    if (!initialFetched.current) {
      initialFetched.current = true;
      void refetch();
    }
    // Reconcile on every extension activate/deactivate so a disabled
    // extension's renderer stops matching new fenced blocks (mirrors the
    // rail / agent picker). `runtime.on` no-ops until the proxy attaches, so
    // re-subscribe on reconnect.
    let offContrib: (() => void) | null = null;
    const subscribeContrib = () => {
      offContrib?.();
      offContrib = runtime.on("extensions/contributions", () => void refetch());
    };
    subscribeContrib();
    const off = browser.on("runtime.reconnected", () => {
      void refetch();
      subscribeContrib();
    });

    return () => {
      cancelled = true;
      off();
      offContrib?.();
    };
  }, []);

  return registry;
}
