/**
 * CriticPassBanner — displayed inline in AgentThreadView when the agent's
 * critic loop produces a critic_result event.
 *
 * supervisor-session-ux task 9.2
 */

import { CheckCircle, XCircle } from "lucide-react";

interface Props {
  passed: boolean;
  summary?: string;
  ts: number;
}

export function CriticPassBanner({ passed, summary, ts }: Props) {
  const label = passed ? "Critic passed" : "Critic requested revision";
  const time = new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

  return (
    <div
      className={`flex items-start gap-2 rounded-md border px-3 py-2 text-sm ${
        passed
          ? "border-green-500/30 bg-green-500/5 text-green-700 dark:text-green-400"
          : "border-amber-500/30 bg-amber-500/5 text-amber-700 dark:text-amber-400"
      }`}
    >
      {passed ? <CheckCircle className="mt-0.5 h-4 w-4 shrink-0" /> : <XCircle className="mt-0.5 h-4 w-4 shrink-0" />}
      <div className="min-w-0 flex-1">
        <div className="flex items-center justify-between gap-2">
          <span className="font-medium">{label}</span>
          <span className="text-xs opacity-60">{time}</span>
        </div>
        {summary && <p className="mt-0.5 text-xs opacity-80">{summary}</p>}
      </div>
    </div>
  );
}
