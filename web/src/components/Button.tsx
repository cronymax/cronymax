import type { ButtonHTMLAttributes, ReactNode } from "react";

type Variant = "ghost" | "primary" | "subtle";

const VARIANTS: Record<Variant, string> = {
  ghost:
    "bg-transparent text-cronymax-caption hover:bg-cronymax-float hover:text-cronymax-title",
  primary: "bg-cronymax-primary text-white hover:bg-cronymax-secondary",
  subtle: "bg-cronymax-float text-cronymax-title hover:bg-cronymax-border",
};

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  children?: ReactNode;
}

export function Button({
  variant = "subtle",
  className = "",
  children,
  ...rest
}: ButtonProps) {
  return (
    <button
      className={[
        "inline-flex items-center justify-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed",
        VARIANTS[variant],
        className,
      ].join(" ")}
      {...rest}
    >
      {children}
    </button>
  );
}

export function IconButton({
  variant = "ghost",
  className = "",
  children,
  ...rest
}: ButtonProps) {
  return (
    <button
      className={[
        "inline-flex h-7 w-7 items-center justify-center rounded-md text-xs transition-colors disabled:opacity-50 disabled:cursor-not-allowed",
        VARIANTS[variant],
        className,
      ].join(" ")}
      {...rest}
    >
      {children}
    </button>
  );
}
