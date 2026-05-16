import { useTheme } from "../../hooks/useTheme";
import type { ThemeMode } from "../../types";

// ── Appearance tab ────────────────────────────────────────────────────────
export function AppearanceTab() {
  const { mode, setMode } = useTheme();
  return (
    <div className="p-4">
      <p className="mb-3 text-xs text-muted-foreground">
        System follows your macOS appearance and switches automatically.
      </p>
      <div className="flex gap-2">
        {(["system", "light", "dark"] as ThemeMode[]).map((m) => (
          <label
            key={m}
            className={`flex-1 cursor-pointer rounded border px-3 py-2 text-center text-xs capitalize transition-colors ${
              mode === m
                ? "border-primary bg-primary/10 text-foreground"
                : "border-border bg-background text-muted-foreground hover:text-foreground"
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
