import type { ButtonHTMLAttributes, ReactNode } from 'react';

type Variant = 'ghost' | 'primary' | 'subtle';

const VARIANTS: Record<Variant, string> = {
  ghost:
    'bg-transparent text-cronymax-fg-muted hover:bg-cronymax-surface-2 hover:text-cronymax-fg',
  primary:
    'bg-cronymax-accent text-white hover:bg-cronymax-accent-soft',
  subtle:
    'bg-cronymax-surface-2 text-cronymax-fg hover:bg-cronymax-border',
};

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  children?: ReactNode;
}

export function Button({
  variant = 'subtle',
  className = '',
  children,
  ...rest
}: ButtonProps) {
  return (
    <button
      className={[
        'inline-flex items-center justify-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed',
        VARIANTS[variant],
        className,
      ].join(' ')}
      {...rest}
    >
      {children}
    </button>
  );
}

export function IconButton({
  variant = 'ghost',
  className = '',
  children,
  ...rest
}: ButtonProps) {
  return (
    <button
      className={[
        'inline-flex h-7 w-7 items-center justify-center rounded-md text-xs transition-colors disabled:opacity-50 disabled:cursor-not-allowed',
        VARIANTS[variant],
        className,
      ].join(' ')}
      {...rest}
    >
      {children}
    </button>
  );
}
