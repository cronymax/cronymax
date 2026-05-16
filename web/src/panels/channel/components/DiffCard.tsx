import type { AppEvent } from "@/types/events";

interface Props {
  event: Extract<AppEvent, { kind: "file_edited" }>;
}

/**
 * DiffCard — renders a `file_edited` event as an inline unified diff.
 *
 * Added lines are shown in green, removed lines in red. The file path
 * is shown in the card header. If no diff is available (e.g. full-file
 * write), the card shows a plain "file written" notice.
 */
export function DiffCard({ event }: Props) {
  const { path, diff } = event.payload;
  const hasDiff = diff.trim().length > 0;

  return (
    <div className="self-start w-full max-w-3xl rounded-lg border border-border bg-background overflow-hidden text-sm">
      <div className="flex items-center gap-2 border-b border-border bg-card px-3 py-1.5">
        <span className="font-mono text-xs truncate opacity-80">{path}</span>
        <span className="ml-auto shrink-0 rounded bg-border px-1.5 py-0.5 text-xs font-medium opacity-70">edited</span>
      </div>

      {hasDiff ? (
        <pre className="overflow-x-auto px-3 py-2 text-xs leading-5 font-mono">
          {parseDiff(diff).map((line, i) => (
            <div
              key={i}
              className={
                line.startsWith("+") && !line.startsWith("+++")
                  ? "bg-green-900/30 text-green-300"
                  : line.startsWith("-") && !line.startsWith("---")
                    ? "bg-red-900/30 text-red-300"
                    : line.startsWith("@@")
                      ? "text-blue-400 opacity-80"
                      : "opacity-70"
              }
            >
              {line}
            </div>
          ))}
        </pre>
      ) : (
        <div className="px-3 py-2 text-xs opacity-60">File written (no diff available)</div>
      )}
    </div>
  );
}

/** Split diff text into lines, preserving empty lines. */
function parseDiff(diff: string): string[] {
  return diff.split("\n");
}
