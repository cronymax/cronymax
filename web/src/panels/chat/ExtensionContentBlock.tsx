// Renders a fenced code block whose language tag matches an installed
// extension content-renderer. Mounts an `<iframe>` pointing at the
// extension's `cronymax-webview://<extId>/<entry>?surface=renderer&id=<inst>`
// URL and drives it via parent→iframe `postMessage` (per P6.5 IDL D9 the
// per-render stream bypasses Rust IPC).
//
// Height adoption (P6.5 IDL gap fix): the iframe is cross-origin so the
// parent can't measure it. The iframe reports its rendered height via
// `acquireCronymaxRendererApi().setHeight(px)` → Rust → Authority topic
// `extensions/renderer`. This component subscribes to that topic and sets
// `<iframe style="height: <px>px">` on matching `HeightChanged` events.

import { useEffect, useMemo, useRef } from "react";
import { runtime } from "../../shells/bridge";
import type { ExtensionRenderer } from "./extensionRenderers";

interface RenderRequest {
  instanceId: string;
  rendererId: string;
  mimeType: string;
  content: string;
  complete: boolean;
  version: number;
  metadata?: {
    language?: string;
    source?: "chat-fence" | "file" | "tool";
    messageId?: string;
  };
}

// Wire shape of every `extensions/renderer` topic event:
//
//   { sequence, emitted_at_ms,
//     payload: {
//       kind: "raw",                              // outer = RuntimeEventPayload variant
//       data: {
//         kind: "heightChanged",                  // inner = RendererEvent variant
//         instanceId: "<uuid>",                   // camelCase via Rust serde rename_all
//         px: 330
//       }
//     }
//   }
//
// The two nesting layers and TWO `kind` fields are easy to confuse. The
// outer one is always `"raw"` (RuntimeEventPayload::Raw); the inner one
// is the actual renderer event discriminator.
interface RendererEventEnvelope {
  payload?: {
    kind?: string;
    data?: {
      kind?: string;
      instanceId?: string;
      px?: number;
    };
  };
}

interface Props {
  /** Resolved extension renderer (extId + entry + mimeTypes). */
  renderer: ExtensionRenderer;
  /** Original fenced-code-block language tag (e.g. "mermaid"). */
  language: string;
  /** Fenced-block content, source-of-truth for the render. */
  content: string;
  /** Optional chat message id this block belongs to. */
  messageId?: string;
  /** Min iframe height before the renderer reports its actual height. */
  initialHeight?: number;
}

function newInstanceId(): string {
  // crypto.randomUUID is broadly available in CEF builds; fall back to a
  // collision-resistant timestamp+random just in case.
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `inst-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

function pickMimeType(renderer: ExtensionRenderer, language: string): string {
  // Prefer a MIME the renderer explicitly declared, in order of
  // specificity. If none match, fall back to whatever the renderer
  // declared first (it accepted the language so it'd better understand).
  const probes = [language, `text/vnd.${language}`, `text/x-${language}`, `application/vnd.${language}`];
  for (const probe of probes) {
    if (renderer.mimeTypes.includes(probe)) return probe;
  }
  return renderer.mimeTypes[0] ?? language;
}

function buildIframeSrc(renderer: ExtensionRenderer, instanceId: string): string {
  const cleanEntry = renderer.entry.replace(/^\.\//, "").replace(/^\/+/, "");
  const encInst = encodeURIComponent(instanceId);
  return `cronymax-webview://${renderer.extId}/${cleanEntry}?surface=renderer&id=${encInst}`;
}

export function ExtensionContentBlock({ renderer, language, content, messageId, initialHeight = 32 }: Props) {
  // One instance per mount; persists across content updates so the
  // iframe doesn't reload on every streaming delta.
  const instanceIdRef = useRef<string>(newInstanceId());
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const versionRef = useRef(0);
  const readyRef = useRef(false);
  const pendingRef = useRef<RenderRequest | null>(null);
  const mimeType = useMemo(() => pickMimeType(renderer, language), [renderer, language]);

  const iframeSrc = useMemo(() => buildIframeSrc(renderer, instanceIdRef.current), [renderer]);
  // Target origin for postMessage — must match the iframe's actual origin
  // (scheme + host = `cronymax-webview://<extId>`) so it reaches the
  // renderer SDK's same-origin guard.
  const targetOrigin = useMemo(() => `cronymax-webview://${renderer.extId}`, [renderer.extId]);

  // Subscribe to height events from the renderer iframe (forwarded via
  // Rust → Authority topic). One subscription per mount; cleaned up on
  // unmount or re-keyed by extId+rendererId (which forces remount via the
  // parent's `key` prop, see ContentStreamView wiring).
  useEffect(() => {
    const off = runtime.on("extensions/renderer", (event) => {
      const env = event as RendererEventEnvelope;
      const inner = env?.payload?.data;
      if (!inner || inner.kind !== "heightChanged") return;
      if (inner.instanceId !== instanceIdRef.current) return;
      const px = typeof inner.px === "number" ? inner.px : 0;
      const el = iframeRef.current;
      if (!el) return;
      // Clamp to a sane upper bound so a buggy extension can't OOM the
      // layout engine by setting Number.MAX_SAFE_INTEGER.
      const clamped = Math.max(0, Math.min(px, 8192));
      el.style.height = `${clamped}px`;
    });
    return () => {
      off?.();
    };
  }, []);

  // Send a render/update to the iframe whenever content changes. The
  // first send fires from `onLoad` (below) once the iframe has acquired
  // its renderer API; subsequent sends fire here directly because the
  // iframe is already alive.
  useEffect(() => {
    versionRef.current += 1;
    const req: RenderRequest = {
      instanceId: instanceIdRef.current,
      rendererId: renderer.rendererId,
      mimeType,
      content,
      complete: true,
      version: versionRef.current,
      metadata: { language, source: "chat-fence", messageId },
    };
    if (!readyRef.current) {
      // Iframe hasn't loaded yet — buffer the latest payload; the
      // onLoad handler will flush it as the initial "render" message.
      pendingRef.current = req;
      return;
    }
    // Once the iframe is loaded, all subsequent payloads use the
    // "update" message type so the renderer can fast-path them via
    // updateItem (falling back to renderItem on the same element if
    // updateItem isn't implemented).
    iframeRef.current?.contentWindow?.postMessage({ type: "cronymax:renderer:update", request: req }, targetOrigin);
  }, [content, mimeType, renderer.rendererId, language, messageId, targetOrigin]);

  // Dispose hook — fires once on unmount, sending the renderer a final
  // dispose message before React strips the iframe.
  useEffect(() => {
    const instId = instanceIdRef.current;
    return () => {
      const win = iframeRef.current?.contentWindow;
      if (win) {
        try {
          win.postMessage({ type: "cronymax:renderer:dispose", instanceId: instId }, targetOrigin);
        } catch {
          /* iframe is already torn down; nothing to do */
        }
      }
    };
  }, [targetOrigin]);

  return (
    <iframe
      ref={iframeRef}
      title={`renderer:${renderer.extId}/${renderer.rendererId}`}
      src={iframeSrc}
      // `allow-scripts allow-same-origin` keeps the iframe's origin equal
      // to its scheme origin (so the renderer SDK's `targetOrigin` check
      // works) while still blocking form submission / popups / top-level
      // navigation. Security relies on the platform-set CSP header, not
      // sandbox.
      sandbox="allow-scripts allow-same-origin"
      style={{
        display: "block",
        width: "100%",
        height: `${initialHeight}px`,
        border: "0",
        background: "transparent",
      }}
      onLoad={() => {
        readyRef.current = true;
        const queued = pendingRef.current;
        pendingRef.current = null;
        const win = iframeRef.current?.contentWindow;
        if (!queued || !win) return;
        win.postMessage({ type: "cronymax:renderer:render", request: queued }, targetOrigin);
      }}
    />
  );
}
