import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
  fallback?: (err: Error) => ReactNode;
}
interface State {
  err: Error | null;
}

/**
 * Top-level error boundary for a panel. In dev mode, renders a visible
 * overlay with the error and stack so Zod validation failures and React
 * exceptions are not swallowed silently.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { err: null };

  static getDerivedStateFromError(err: Error): State {
    return { err };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    // eslint-disable-next-line no-console
    console.error("[ErrorBoundary]", error, info.componentStack);
  }

  render(): ReactNode {
    if (this.state.err) {
      if (this.props.fallback) return this.props.fallback(this.state.err);
      return (
        <div className="m-4 rounded-popover border border-cronymax-error bg-cronymax-base p-4 text-cronymax-title">
          <h3 className="text-sm font-semibold text-cronymax-error mb-2">panel error</h3>
          <pre className="text-xs whitespace-pre-wrap font-mono text-cronymax-caption">
            {this.state.err.message}
            {this.state.err.stack ? `\n\n${this.state.err.stack}` : ""}
          </pre>
        </div>
      );
    }
    return this.props.children;
  }
}
