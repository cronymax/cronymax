import { useCallback, useEffect, useState } from "react";
import { inputCls } from "../panels/flows/App";

// ── Providers tab ─────────────────────────────────────────────────────────
type ProviderKind = "openai" | "anthropic" | "ollama" | "github_copilot" | "custom";

export interface LlmProvider {
  id: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  api_key: string;
  default_model: string;
}
const ANTHROPIC_MODELS = [
  "claude-opus-4-5",
  "claude-sonnet-4-5",
  "claude-3-5-sonnet-latest",
  "claude-3-5-haiku-latest",
  "claude-3-opus-20240229",
];

async function listProviderModels(provider: LlmProvider): Promise<string[]> {
  const { kind, base_url, api_key } = provider;
  if (!base_url) return [];
  if (kind === "anthropic") return ANTHROPIC_MODELS;
  if (kind === "ollama") {
    const base = base_url.replace(/\/v1\/?$/, "");
    const res = await fetch(`${base}/api/tags`, {
      signal: AbortSignal.timeout(8000),
    });
    if (!res.ok) throw new Error(`/api/tags ${res.status}`);
    const data = await res.json();
    return ((data.models ?? []) as { name: string }[]).map((m) => m.name).sort();
  }
  const headers: Record<string, string> = { Accept: "application/json" };
  if (api_key) headers.Authorization = `Bearer ${api_key}`;
  const url = `${base_url.replace(/\/?$/, "")}/models`;
  const res = await fetch(url, { headers, signal: AbortSignal.timeout(8000) });
  if (!res.ok) throw new Error(`/models ${res.status}`);
  const data = await res.json();
  return ((data.data ?? []) as { id: string }[]).map((m) => m.id).sort();
}

export function ModelSelect({
  value,
  onChange,
  provider,
}: {
  value: string;
  onChange: (v: string) => void;
  provider: LlmProvider;
}) {
  const [models, setModels] = useState<string[]>([]);
  const [fetching, setFetching] = useState(false);
  const [fetchErr, setFetchErr] = useState<string | null>(null);

  const doFetch = useCallback(async () => {
    setFetching(true);
    setFetchErr(null);
    try {
      const list = await listProviderModels(provider);
      setModels(list);
    } catch (e) {
      setFetchErr((e as Error).message);
    } finally {
      setFetching(false);
    }
  }, [provider.kind, provider.base_url, provider.api_key]);

  useEffect(() => {
    if (provider.base_url) void doFetch();
  }, [provider.kind, provider.base_url]);

  return (
    <div className="space-y-1">
      <div className="flex gap-1">
        <input
          className={`${inputCls} flex-1`}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          list="model-datalist"
          placeholder="gpt-4o-mini"
        />
        <datalist id="model-datalist">
          {models.map((m) => (
            <option key={m} value={m} />
          ))}
        </datalist>
        <button
          type="button"
          onClick={() => void doFetch()}
          disabled={fetching || !provider.base_url}
          title="Fetch available models"
          className="rounded border border-cronymax-border px-2 text-xs hover:bg-cronymax-float disabled:opacity-40"
        >
          {fetching ? "…" : "⟳"}
        </button>
      </div>
      {fetchErr && <p className="text-[11px] text-red-400">fetch failed: {fetchErr}</p>}
      {models.length > 0 && !fetching && (
        <div className="flex max-h-[120px] flex-wrap gap-1 overflow-y-auto pt-0.5">
          {models.map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => onChange(m)}
              className={
                "rounded border px-1.5 py-0.5 text-[11px] " +
                (m === value
                  ? "border-cronymax-primary bg-cronymax-primary/20 text-cronymax-title"
                  : "border-cronymax-border text-cronymax-caption hover:text-cronymax-title")
              }
            >
              {m}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
