import type { AppEvent } from "../../../types/events";

type GitCommitEvent = Extract<AppEvent, { kind: "git_commit_created" }>;

interface Props {
  event: GitCommitEvent;
}

export function GitCommitCard({ event }: Props) {
  const { hash, message, files_changed } = event.payload;
  const shortHash = hash.slice(0, 8);

  return (
    <div
      style={{
        borderRadius: 6,
        border: "1px solid var(--vscode-editorWidget-border, #444)",
        overflow: "hidden",
        fontSize: 12,
        fontFamily: "var(--vscode-editor-font-family, monospace)",
        marginBottom: 8,
      }}
    >
      {/* Header */}
      <div
        style={{
          background:
            "var(--vscode-editor-inactiveSelectionBackground, #2d2d2d)",
          padding: "4px 10px",
          display: "flex",
          alignItems: "center",
          gap: 8,
          color: "var(--vscode-foreground, #ccc)",
        }}
      >
        <span style={{ opacity: 0.6 }}>git commit</span>
        <code style={{ color: "var(--vscode-textLink-foreground, #4ec9b0)" }}>
          {shortHash}
        </code>
        <span
          style={{
            flex: 1,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {message}
        </span>
      </div>

      {/* Files changed */}
      {files_changed.length > 0 && (
        <div
          style={{
            padding: "4px 10px",
            background: "var(--vscode-editor-background, #1e1e1e)",
          }}
        >
          {files_changed.map((f) => (
            <div
              key={f}
              style={{
                color:
                  "var(--vscode-gitDecoration-modifiedResourceForeground, #e2c08d)",
              }}
            >
              {f}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
