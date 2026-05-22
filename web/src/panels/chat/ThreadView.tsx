/**
 * ThreadView — top-level wrapper that dispatches to `AgentThreadView` or
 * `FlowThreadView` based on the active `threadView.kind`.
 *
 * supervisor-session-ux task 7.3
 */

import { AgentThreadView } from "./AgentThreadView";
import { FlowThreadView } from "./FlowThreadView";
import type { ThreadViewTarget } from "./store";

// ── Component ──────────────────────────────────────────────────────────────

interface Props {
  threadView: ThreadViewTarget;
}

export function ThreadView({ threadView }: Props) {
  if (threadView.kind === "agent") {
    return <AgentThreadView taskId={threadView.taskId} />;
  }
  if (threadView.kind === "flow") {
    return <FlowThreadView taskId={threadView.taskId} />;
  }
  // exhaustive never guard
  ((_x: never) => {})(threadView.kind);
  return null;
}
