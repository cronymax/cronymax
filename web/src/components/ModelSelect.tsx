import { Check, ChevronsUpDown } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";

// ── Providers tab ─────────────────────────────────────────────────────────
type ProviderKind = "openai" | "anthropic" | "ollama" | "github_copilot" | "custom";

export interface LlmProvider {
  id: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  api_key: string;
  default_model: string;
  /** Default reasoning_effort for runs against this provider. Empty = none. */
  reasoning_effort?: string;
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
  const [comboOpen, setComboOpen] = useState(false);

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
        <Popover open={comboOpen} onOpenChange={setComboOpen}>
          <PopoverTrigger asChild>
            <Button type="button" variant="outline" className="flex-1 h-7 justify-between text-xs font-normal truncate">
              <span className="truncate">{value || "Select model…"}</span>
              <ChevronsUpDown size={12} className="ml-1 shrink-0 opacity-50" />
            </Button>
          </PopoverTrigger>
          <PopoverContent className="p-0 w-[280px]" align="start">
            <Command>
              <CommandInput placeholder="Search models…" className="h-8 text-xs" />
              <CommandList>
                <CommandEmpty>No models found.</CommandEmpty>
                <CommandGroup>
                  {models.map((m) => (
                    <CommandItem
                      key={m}
                      value={m}
                      onSelect={(v) => {
                        onChange(v);
                        setComboOpen(false);
                      }}
                      className="text-xs"
                    >
                      <Check size={12} className={cn("mr-2 shrink-0", m === value ? "opacity-100" : "opacity-0")} />
                      <span className="font-mono truncate">{m}</span>
                    </CommandItem>
                  ))}
                </CommandGroup>
              </CommandList>
            </Command>
          </PopoverContent>
        </Popover>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void doFetch()}
          disabled={fetching || !provider.base_url}
          title="Fetch available models"
          className="px-2 text-xs"
        >
          {fetching ? "…" : "⟳"}
        </Button>
      </div>
      {fetchErr && <p className="text-xs text-red-400">fetch failed: {fetchErr}</p>}
    </div>
  );
}
