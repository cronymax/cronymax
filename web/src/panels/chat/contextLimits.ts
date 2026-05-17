/**
 * Known context-window sizes (in tokens) for common model families.
 *
 * Used by the Context Window Hint bar to show how much of the context
 * the current conversation has consumed.
 *
 * Entries are intentionally conservative; unknown models surface no hint.
 */
export const MODEL_CONTEXT_LIMITS: Record<string, number> = {
  // OpenAI
  "gpt-4o": 128_000,
  "gpt-4o-mini": 128_000,
  "gpt-4-turbo": 128_000,
  "gpt-4": 8_192,
  "gpt-3.5-turbo": 16_385,
  o1: 200_000,
  "o1-mini": 128_000,
  o3: 200_000,
  "o3-mini": 200_000,
  "o4-mini": 200_000,
  "gpt-5": 200_000,

  // Anthropic
  "claude-3-5-sonnet": 200_000,
  "claude-3-5-haiku": 200_000,
  "claude-3-opus": 200_000,
  "claude-3-haiku": 200_000,
  "claude-3-sonnet": 200_000,
  "claude-sonnet-4": 200_000,
  "claude-opus-4": 200_000,

  // Google
  "gemini-1.5-pro": 2_000_000,
  "gemini-1.5-flash": 1_000_000,
  "gemini-2.0-flash": 1_000_000,
  "gemini-2.5-pro": 2_000_000,
  "gemini-2.5-flash": 1_000_000,
};

/**
 * Resolve the context limit for a given model string.
 * Matches by exact key first, then by prefix (e.g. "claude-3-5-sonnet-20241022").
 */
export function resolveContextLimit(model: string): number | null {
  if (!model) return null;
  if (model in MODEL_CONTEXT_LIMITS) return MODEL_CONTEXT_LIMITS[model] ?? null;
  // Prefix match — try longest matching key first
  const keys = Object.keys(MODEL_CONTEXT_LIMITS).sort((a, b) => b.length - a.length);
  for (const key of keys) {
    if (model.startsWith(key)) return MODEL_CONTEXT_LIMITS[key] ?? null;
  }
  return null;
}
