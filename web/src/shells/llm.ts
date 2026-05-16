/** Anthropic API contract version pinned in the `anthropic-version`
 * request header. Required by `/v1/*` endpoints; not related to the
 * model version. Keep in sync with `ANTHROPIC_API_VERSION` in
 * `crates/cronymax/src/llm/anthropic.rs`.
 *
 * See https://docs.anthropic.com/en/api/versioning
 */
export const ANTHROPIC_API_VERSION = "2023-06-01";

export type ProviderKind = "openai" | "anthropic" | "ollama" | "github_copilot" | "custom";

/** Minimal shape needed to list a provider's models. */
export interface ProviderEndpoint {
  kind: ProviderKind;
  base_url: string;
  api_key: string;
}

/** Static fallback when an Anthropic-compatible endpoint doesn't implement
 * `GET /v1/models`. Keep current models near the top. */
export const ANTHROPIC_FALLBACK = [
  "claude-opus-4-7",
  "claude-opus-4-6",
  "claude-sonnet-4-6",
  "claude-opus-4-5",
  "claude-sonnet-4-5",
  "claude-haiku-4-5",
];

/** Static fallback for GitHub Copilot endpoints that don't return `/models`. */
export const COPILOT_FALLBACK = ["gpt-4o", "gpt-4o-mini", "claude-3.5-sonnet", "o3-mini"];

/** Parse an OpenAI-style `{ data: [{ id }, …] }` response, surfacing a clear
 *  error if the body wasn't JSON (e.g. a proxy returned an HTML login page). */
async function parseModelsResponse(res: Response): Promise<string[]> {
  const text = await res.text();
  let data: { data?: { id: string }[] };
  try {
    data = JSON.parse(text) as { data?: { id: string }[] };
  } catch {
    const preview = text.trim().slice(0, 80).replace(/\s+/g, " ");
    throw new Error(`expected JSON, got: ${preview}`);
  }
  return (data.data ?? []).map((m) => m.id).sort();
}

/** Fetch the model list for a provider, dispatching by `kind`. Paths follow
 * the official OpenAI / Anthropic specs (always `/v1/...`), since real
 * proxies (sub.redhood.dev, etc.) reject the non-prefixed paths and return
 * HTML landing pages:
 *  - anthropic → `GET {base}/v1/models` (Anthropic headers). Silent fallback to ANTHROPIC_FALLBACK on error / empty.
 *  - ollama → `GET {base}/api/tags` (strips trailing `/v1`). THROWS on error.
 *  - openai / github_copilot / custom → `GET {base}/v1/models` (Bearer auth). THROWS on error (github_copilot silently falls back to COPILOT_FALLBACK).
 *
 * `timeoutMs` defaults to 8000.
 */
export async function listProviderModels(provider: ProviderEndpoint, timeoutMs = 8000): Promise<string[]> {
  const { kind, base_url, api_key } = provider;
  if (!base_url) return [];
  const base = base_url.replace(/\/+$/, "");

  if (kind === "anthropic") {
    const headers: Record<string, string> = {
      Accept: "application/json",
      "anthropic-version": ANTHROPIC_API_VERSION,
    };
    if (api_key) headers["x-api-key"] = api_key;
    try {
      const res = await fetch(`${base}/v1/models`, {
        headers,
        signal: AbortSignal.timeout(timeoutMs),
      });
      if (res.ok) {
        const list = await parseModelsResponse(res);
        if (list.length > 0) return list;
      }
    } catch {
      /* fall through to static fallback */
    }
    return ANTHROPIC_FALLBACK;
  }

  if (kind === "ollama") {
    const ollamaBase = base.replace(/\/v1$/, "");
    const res = await fetch(`${ollamaBase}/api/tags`, {
      signal: AbortSignal.timeout(timeoutMs),
    });
    if (!res.ok) throw new Error(`/api/tags ${res.status}`);
    const data = (await res.json()) as { models?: { name: string }[] };
    return (data.models ?? []).map((m) => m.name).sort();
  }

  // openai / github_copilot / custom — official OpenAI path is `/v1/models`.
  const headers: Record<string, string> = { Accept: "application/json" };
  if (api_key) headers.Authorization = `Bearer ${api_key}`;
  const url = `${base}/v1/models`;

  if (kind === "github_copilot") {
    try {
      const res = await fetch(url, {
        headers,
        signal: AbortSignal.timeout(timeoutMs),
      });
      if (res.ok) {
        const list = await parseModelsResponse(res);
        if (list.length > 0) return list;
      }
    } catch {
      /* fall through to fallback */
    }
    return COPILOT_FALLBACK;
  }

  // openai / custom — surface errors instead of silently returning [].
  const res = await fetch(url, {
    headers,
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!res.ok) throw new Error(`/v1/models ${res.status}`);
  return parseModelsResponse(res);
}
