import { Check, ChevronsUpDown } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import { listProviderModels, type ProviderKind } from "@/shells/llm";

// ── Providers tab ─────────────────────────────────────────────────────────

export interface LlmProvider {
  id: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  api_key: string;
  default_model: string;
  /** Default reasoning_effort for OpenAI-style runs against this provider
   * (gpt-5 / o-series). Empty = none. */
  reasoning_effort?: string;
  /** Default Anthropic adaptive-thinking effort (claude-* models).
   * Values: "" | "low" | "medium" | "high" | "max". Empty = none. */
  anthropic_effort?: string;
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

  // Always include the currently-selected value so it remains visible in the
  // list even if it was set manually or the live fetch returned a shorter list.
  const displayModels = value && !models.includes(value) ? [value, ...models] : models;

  useEffect(() => {
    if (provider.base_url) void doFetch();
  }, [provider.kind, provider.base_url]);

  return (
    <div className="space-y-1">
      <div className="flex gap-1">
        <Popover open={comboOpen} onOpenChange={setComboOpen}>
          <PopoverTrigger asChild>
            <Button type="button" variant="outline" size="sm" className="w-full justify-between font-normal">
              <span className="truncate">{value || "Select model…"}</span>
              <ChevronsUpDown className="shrink-0 opacity-50" />
            </Button>
          </PopoverTrigger>
          <PopoverContent className="p-0 w-[280px]" align="start">
            <Command>
              <CommandInput placeholder="Search models…" />
              <CommandList>
                <CommandEmpty>No models found.</CommandEmpty>
                <CommandGroup>
                  {displayModels.map((m) => (
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
