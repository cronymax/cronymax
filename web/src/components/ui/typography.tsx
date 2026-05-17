import * as LabelPrimitive from "@radix-ui/react-label";
import * as React from "react";

import { cn } from "@/lib/utils";

/** Section / dialog title — `text-sm font-semibold`. */
const Heading = React.forwardRef<HTMLHeadingElement, React.HTMLAttributes<HTMLHeadingElement>>(
  ({ className, ...props }, ref) => (
    <h2 ref={ref} className={cn("text-sm font-semibold leading-none", className)} {...props} />
  ),
);
Heading.displayName = "Heading";

/** Muted caption text — descriptions, hints, helper text. */
const Caption = React.forwardRef<HTMLParagraphElement, React.HTMLAttributes<HTMLParagraphElement>>(
  ({ className, ...props }, ref) => (
    <p ref={ref} className={cn("text-xs text-muted-foreground", className)} {...props} />
  ),
);
Caption.displayName = "Caption";

/** Uppercase label above a form input (built on Radix Label primitive). */
const FieldLabel = React.forwardRef<
  React.ElementRef<typeof LabelPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof LabelPrimitive.Root>
>(({ className, ...props }, ref) => (
  <LabelPrimitive.Root
    ref={ref}
    className={cn("block text-xs font-medium uppercase tracking-wide text-muted-foreground", className)}
    {...props}
  />
));
FieldLabel.displayName = "FieldLabel";

/** Form error / destructive helper text. */
const ErrorText = React.forwardRef<HTMLParagraphElement, React.HTMLAttributes<HTMLParagraphElement>>(
  ({ className, ...props }, ref) => <p ref={ref} className={cn("text-xs text-destructive", className)} {...props} />,
);
ErrorText.displayName = "ErrorText";

/** Inline monospace code-like text. */
const InlineCode = React.forwardRef<HTMLElement, React.HTMLAttributes<HTMLElement>>(({ className, ...props }, ref) => (
  <code ref={ref} className={cn("rounded bg-muted px-1.5 py-0.5 font-mono text-xs", className)} {...props} />
));
InlineCode.displayName = "InlineCode";

export { Caption, ErrorText, FieldLabel, Heading, InlineCode };
