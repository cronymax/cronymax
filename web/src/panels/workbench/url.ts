/**
 * URL parsing for the document workbench.
 *
 * Surface contract (`web/public/document-workbench.html`):
 *   ?flow=<flow_id>
 *   &doc=<doc_name>           // bare name without `.md` suffix
 *   &mode=wysiwyg|source|diff // default: wysiwyg
 *   &run_id=<run_id>          // required for comment rail / suggested edits
 *   &from=<rev>&to=<rev>      // diff mode only
 *   #block-<uuid>             // optional deep-link to a block
 */
export type WorkbenchMode = "wysiwyg" | "source" | "diff";

export interface WorkbenchParams {
  flow: string;
  doc: string;
  mode: WorkbenchMode;
  /** Active flow run; required for any review/comment interaction. */
  runId?: string;
  from?: number;
  to?: number;
  blockId?: string;
}

export function readParams(): WorkbenchParams {
  const p = new URLSearchParams(window.location.search);
  const flow = p.get("flow") ?? "";
  const doc = p.get("doc") ?? "";
  const m = p.get("mode");
  const mode: WorkbenchMode = m === "source" || m === "diff" ? m : "wysiwyg";
  const runId = p.get("run_id") ?? undefined;
  const fromStr = p.get("from");
  const toStr = p.get("to");
  const from = fromStr ? Number(fromStr) : undefined;
  const to = toStr ? Number(toStr) : undefined;
  const hash = window.location.hash;
  const blockId = hash.startsWith("#block-")
    ? hash.slice("#block-".length)
    : undefined;
  return {
    flow,
    doc,
    mode,
    runId: runId || undefined,
    from: Number.isFinite(from) ? from : undefined,
    to: Number.isFinite(to) ? to : undefined,
    blockId,
  };
}

export function setMode(
  mode: WorkbenchMode,
  extra?: Record<string, string>,
): void {
  const url = new URL(window.location.href);
  url.searchParams.set("mode", mode);
  if (extra) {
    for (const [k, v] of Object.entries(extra)) url.searchParams.set(k, v);
  } else {
    url.searchParams.delete("from");
    url.searchParams.delete("to");
  }
  window.history.replaceState(null, "", url.toString());
}
