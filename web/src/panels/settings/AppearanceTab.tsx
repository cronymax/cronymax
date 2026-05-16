import { useTheme } from "../../hooks/useTheme";
import type { ThemeMode } from "../../types";

// ── Appearance tab ────────────────────────────────────────────────────────
export function AppearanceTab() {
  const { mode, setMode } = useTheme();
  return (
    <div className="p-4">
      <p className="mb-3 text-xs text-cronymax-caption">
        System follows your macOS appearance and switches automatically.
      </p>
      <div className="flex gap-2">
        {(["system", "light", "dark"] as ThemeMode[]).map((m) => (
          <label
            key={m}
            className={`flex-1 cursor-pointer rounded border px-3 py-2 text-center text-xs capitalize transition-colors ${
              mode === m
                ? "border-cronymax-primary bg-cronymax-primary/10 text-cronymax-title"
                : "border-cronymax-border bg-cronymax-base text-cronymax-caption hover:text-cronymax-title"
            }`}
          >
            <input
              type="radio"
              name="theme-mode"
              value={m}
              checked={mode === m}
              onChange={() => setMode(m)}
              className="sr-only"
            />
            {m}
          </label>
        ))}
      </div>
    </div>
  );
}
