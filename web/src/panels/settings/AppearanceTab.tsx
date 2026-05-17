import { CircleCheck } from "lucide-react";
import { Card, CardFooter } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Caption } from "@/components/ui/typography";
import { cn } from "@/lib/utils";
import { useTheme } from "../../hooks/useTheme";
import type { ThemeMode } from "../../types";

// ── Appearance tab ────────────────────────────────────────────────────────
const MODES: ThemeMode[] = ["system", "light", "dark"];

interface PreviewColors {
  bg: string;
  card: string;
  fg: string;
  muted: string;
  border: string;
}

/* Hardcoded hex preview colours mirroring theme.css.
 * Cannot use semantic Tailwind tokens — each preview must always render
 * in the colours of the theme it REPRESENTS, independent of the user's
 * currently active mode. */
const PREVIEW: Record<"light" | "dark", PreviewColors> = {
  light: {
    bg: "#ffffff",
    card: "#f7f7f7",
    fg: "#252525",
    muted: "#a3a3a3",
    border: "#e5e5e5",
  },
  dark: {
    bg: "#252525",
    card: "#363636",
    fg: "#fafafa",
    muted: "#717171",
    border: "rgba(255, 255, 255, 0.12)",
  },
};
const PRIMARY = "#0f766e"; // teal-700 brand

function PreviewPane({ colors }: { colors: PreviewColors }) {
  // Every visible surface uses inline hex strings — no semantic tokens,
  // no Tailwind colour utilities — so this block renders identically
  // regardless of the user's currently active theme.
  return (
    <div className="flex h-full w-full flex-col" style={{ backgroundColor: colors.bg }}>
      <div
        className="flex h-4 shrink-0 items-center gap-1 px-2"
        style={{ backgroundColor: colors.card, borderBottomWidth: 1, borderBottomColor: colors.border }}
      >
        <div className="size-1.5 rounded-full" style={{ backgroundColor: colors.muted }} />
        <div className="size-1.5 rounded-full" style={{ backgroundColor: colors.muted }} />
        <div className="size-1.5 rounded-full" style={{ backgroundColor: colors.muted }} />
      </div>
      <div className="flex flex-1 flex-col justify-between p-2">
        <div className="flex flex-col gap-1">
          <div className="h-1.5 w-12 rounded-sm" style={{ backgroundColor: colors.fg }} />
          <div className="h-1 w-full rounded-sm" style={{ backgroundColor: colors.muted }} />
          <div className="h-1 w-4/5 rounded-sm" style={{ backgroundColor: colors.muted }} />
        </div>
        <div className="h-2 w-10 rounded-sm" style={{ backgroundColor: PRIMARY }} />
      </div>
    </div>
  );
}

function ThemePreview({ mode }: { mode: ThemeMode }) {
  if (mode === "system") {
    return (
      <div className="grid h-28 w-full grid-cols-2">
        <PreviewPane colors={PREVIEW.light} />
        <PreviewPane colors={PREVIEW.dark} />
      </div>
    );
  }
  return (
    <div className="h-28 w-full">
      <PreviewPane colors={PREVIEW[mode]} />
    </div>
  );
}

export function AppearanceTab() {
  const { mode, setMode } = useTheme();
  return (
    <div className="p-4">
      <Caption className="mb-3">System follows your macOS appearance and switches automatically.</Caption>
      <RadioGroup value={mode} onValueChange={(v) => setMode(v as ThemeMode)} className="grid grid-cols-3 gap-4">
        {MODES.map((m) => {
          const id = `theme-mode-${m}`;
          const selected = mode === m;
          return (
            <Label key={m} htmlFor={id} className="block min-w-0 cursor-pointer">
              <RadioGroupItem id={id} value={m} className="sr-only" />
              <Card
                className={cn(
                  "w-full gap-0 overflow-hidden py-0 transition-all",
                  selected ? "ring-2 ring-primary" : "hover:ring-2 hover:ring-foreground/30",
                )}
              >
                <ThemePreview mode={m} />
                <CardFooter className="justify-between p-3">
                  <span className="text-xs font-medium capitalize">{m}</span>
                  <CircleCheck
                    className={cn("size-5 fill-primary text-primary-foreground", !selected && "invisible")}
                    aria-hidden={!selected}
                  />
                </CardFooter>
              </Card>
            </Label>
          );
        })}
      </RadioGroup>
    </div>
  );
}
