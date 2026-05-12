import type { AppEvent } from "../../../types/events";

type GitPushedEvent = Extract<AppEvent, { kind: "git_pushed" }>;

interface Props {
  event: GitPushedEvent;
}

export function GitPushedCard({ event }: Props) {
  const { remote, branch, commits_pushed } = event.payload;

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
        <span style={{ opacity: 0.6 }}>git push</span>
        <code style={{ color: "var(--vscode-textLink-foreground, #4ec9b0)" }}>
          {remote}/{branch}
        </code>
        <span
          style={{
            color:
              "var(--vscode-gitDecoration-addedResourceForeground, #81b88b)",
          }}
        >
          {commits_pushed} commit{commits_pushed !== 1 ? "s" : ""}
        </span>
      </div>
    </div>
  );
}
