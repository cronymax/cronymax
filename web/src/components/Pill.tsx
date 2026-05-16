import type { ReactNode } from "react";

export function Pill({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div className={["flex items-center gap-2 rounded-pill bg-cronymax-float px-3 py-1 text-xs", className].join(" ")}>
      {children}
    </div>
  );
}
