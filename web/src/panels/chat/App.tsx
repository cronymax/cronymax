import {
  ArrowUp,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronsUpDown,
  Copy,
  GitFork,
  Image as ImageIcon,
  Info,
  Loader2,
  MessageSquare,
  Paperclip,
  Pin,
  Plus,
  Square,
  TriangleAlert,
  Undo2,
  X,
} from "lucide-react";
import {
  type ChangeEvent,
  type ClipboardEvent,
  type FormEvent,
  type KeyboardEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { FlowInstancesBar } from "@/components/FlowInstancesBar";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverAnchor, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Heading } from "@/components/ui/typography";
import { useRuntimeEvent } from "@/hooks/useRuntimeEvent";
import { cn } from "@/lib/utils";
import { browser, shells } from "@/shells/bridge";
import { listProviderModels } from "@/shells/llm";
import { agentRegistry, agentRun, b64ToUtf8, flowRun, terminal as rt_terminal } from "@/shells/runtime";
import { ApprovalCard, loadTrustMap } from "./ApprovalCard";
import { ContentStreamView } from "./ContentStreamView";
import { resolveContextLimit } from "./contextLimits";
import { FileChangesView } from "./FileChangesView";
import { LiveTasksView } from "./LiveTasksView";
import { PromptPopover } from "./PromptPopover";
import { ReviewsPanel } from "./ReviewsPanel";
import {
  type AnthropicEffort,
  type Attachment,
  type Block,
  type ConversationBlock,
  chatNameFor,
  ensureChat,
  type FileChange,
  loadAnthropicEffort,
  loadChatData,
  loadChatsList,
  loadFlowsList,
  loadReasoningEffort,
  loadSelectedModel,
  persistAnthropicEffort,
  persistChatData,
  persistReasoningEffort,
  persistSelectedFlow,
  persistSelectedModel,
  type ReasoningEffort,
  type ShellBlock,
  type Thread,
  useStore,
} from "./store";
import { TraceViewer } from "./TraceViewer";
import { useSelectionTooltip } from "./useSelectionTooltip";

// ── File mutation detection ─────────────────────────────────────────────

/** Tools that write files to the workspace and the field containing the path. */
const MUTATION_TOOLS: Record<string, { operation: FileChange["operation"]; pathField: string }> = {
  write_file: { operation: "created", pathField: "path" },
  edit_file: { operation: "modified", pathField: "path" },
  patch_file: { operation: "modified", pathField: "path" },
  create_file: { operation: "created", pathField: "path" },
  delete_file: { operation: "deleted", pathField: "path" },
  move_file: { operation: "moved", pathField: "source_path" },
  rename_file: { operation: "moved", pathField: "path" },
  create_directory: { operation: "created", pathField: "path" },
  overwrite_file: { operation: "modified", pathField: "path" },
  // Agent-level tool names may use snake_case prefixed variants
  file_write: { operation: "created", pathField: "path" },
  file_edit: { operation: "modified", pathField: "path" },
  file_delete: { operation: "deleted", pathField: "path" },
};

function detectFileChange(tool: string, args: unknown): { path: string; operation: FileChange["operation"] } | null {
  const def = MUTATION_TOOLS[tool];
  if (!def) return null;
  if (!args || typeof args !== "object") return null;
  const a = args as Record<string, unknown>;
  const path = typeof a[def.pathField] === "string" ? (a[def.pathField] as string) : null;
  if (!path) return null;
  return { path, operation: def.operation };
}

// ── shadcn Tooltip helper ──────────────────────────────────────────────
// Wraps any element with a shadcn Tooltip so callers can replace
// `title="..."` (browser-native tooltip — system styled, instant,
// inconsistent with the rest of the UI) with the same theme-aware
// tooltip used elsewhere. Pass plain text via `tip`; the child becomes
// the trigger via `asChild`. A top-level `<TooltipProvider>` wraps
// `<main>` so all instances share the same delay timer.
function Tip({ tip, children }: { tip: string; children: React.ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent>{tip}</TooltipContent>
    </Tooltip>
  );
}

// ── picker types ────────────────────────────────────────────────────────

interface PickerItem {
  id: string;
  /** Short label shown in bold */
  label: string;
  /** Optional description shown in lighter text */
  description?: string;
  /** For slash commands: built-in action name */
  action?: "clear" | "new";
  /** For slash prompts: text to insert as user message */
  content?: string;
}

interface PickerState {
  type: "slash" | "at";
  /** The raw text typed after the trigger character (e.g. "cl" after "/cl") */
  query: string;
  /** Caret offset at which the trigger started, so we can splice the replacement */
  triggerStart: number;
}

/**
 * Strip YAML front-matter from a .prompt.md file and return the
 * human-readable description (from the `description:` field) plus the
 * cleaned body to use as prompt content.
 */
function parseFrontmatter(raw: string): {
  description?: string;
  content: string;
} {
  if (!raw.startsWith("---\n") && !raw.startsWith("---\r\n")) return { content: raw };
  const end = raw.indexOf("\n---", 4);
  if (end === -1) return { content: raw };
  const fm = raw.slice(4, end);
  // raw[end]='\n', raw[end+1..end+3]='---'; slice past them and strip leading blank lines.
  const afterClose = raw.slice(end + 4).replace(/^[\r\n]+/, "");
  const descMatch = fm.match(/^description:\s*(.+)$/m);
  return { description: descMatch?.[1]?.trim(), content: afterClose };
}

/**
 * Render a user message string, wrapping `/slug` tokens (prompt pill
 * references) as visually distinct inline badges.
 */
function UserMessageContent({ text, onPillClick }: { text: string; onPillClick?: (label: string) => void }) {
  const pillRe = /^\/[a-z0-9_][a-z0-9_-]*$/i;
  // Split preserving whitespace separators so we can re-render them as-is.
  const words = text.split(/([ \t]+)/);
  return (
    <>
      {words.map((word, i) =>
        pillRe.test(word) ? (
          <button
            key={i}
            type="button"
            onClick={() => onPillClick?.(word.slice(1))}
            className="mr-0.5 inline-flex cursor-pointer items-center rounded-md border border-primary/30 bg-primary/15 px-1.5 py-0 align-middle font-mono text-xs text-primary transition-colors hover:bg-primary/25"
          >
            <span className="opacity-60">/</span>
            {word.slice(1)}
          </button>
        ) : (
          <span key={i}>{word}</span>
        ),
      )}
    </>
  );
}

/** Reads custom slash-command prompts stored in localStorage. */
function loadCustomPrompts(): PickerItem[] {
  try {
    const raw = localStorage.getItem("cronymax.custom_prompts");
    if (!raw) return [];
    const arr = JSON.parse(raw) as Array<{
      id?: string;
      title?: string;
      content?: string;
    }>;
    return arr
      .filter((x) => x.content)
      .map((x, i) => ({
        id: x.id ?? `custom-${i}`,
        label: x.title ?? `Prompt ${i + 1}`,
        description: x.content?.slice(0, 60),
        content: x.content,
      }));
  } catch {
    return [];
  }
}

/** Built-in slash commands (always shown, not user-configurable). */
const BUILTIN_COMMANDS: PickerItem[] = [
  {
    id: "cmd-clear",
    label: "clear",
    description: "Clear chat history",
    action: "clear",
  },
  {
    id: "cmd-new",
    label: "new",
    description: "Start a new chat",
    action: "new",
  },
];

// ── helpers ────────────────────────────────────────────────────────────

function parseMention(text: string, agents: string[]): { agent: string | null; body: string } {
  const m = text.match(/^@([A-Za-z0-9_.-]+)\s*(.*)$/s);
  if (!m) return { agent: null, body: text };
  const want = m[1]!.toLowerCase();
  const hit = agents.find((a) => a.toLowerCase() === want);
  return hit ? { agent: hit, body: m[2] ?? "" } : { agent: null, body: text };
}

function fmtDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

// ── Block components ────────────────────────────────────────────────────

function ThreadSummary({ thread, onExpand }: { thread: Thread; onExpand: () => void }) {
  const lastMsg = thread.messages.at(-1);
  return (
    <Card size="sm" className="mt-2 text-xs">
      <CardContent className="flex flex-col gap-1">
        <div className="flex items-center gap-2">
          <span className="font-semibold capitalize text-primary">{thread.action}</span>
          <span className="text-muted-foreground">
            {thread.messages.length} message
            {thread.messages.length !== 1 ? "s" : ""}
          </span>
          {thread.running && <span className="italic text-muted-foreground">running…</span>}
          <Button variant="link" size="sm" onClick={onExpand} className="ml-auto h-auto p-0">
            View thread
          </Button>
        </div>
        {lastMsg && (
          <div className="truncate text-muted-foreground">
            {lastMsg.role === "assistant" ? lastMsg.content.slice(0, 80) : ""}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function ConversationBlockView({
  block,
  isStreaming,
  isHighlighted,
  workspacePrompts = [],
  onRestore,
  onFork,
}: {
  block: ConversationBlock;
  isStreaming: boolean;
  isHighlighted?: boolean;
  workspacePrompts?: PickerItem[];
  onRestore?: (blockId: string) => void;
  onFork?: (blockId: string) => void;
}) {
  const pinnedComments = block.comments.filter((c) => c.pinnedToPrompt);
  const [activePillLabel, setActivePillLabel] = useState<string | null>(null);
  const [hovered, setHovered] = useState(false);
  const activePillPrompt = activePillLabel ? workspacePrompts.find((p) => p.label === activePillLabel) : null;
  return (
    <div
      className={cn(
        "flex flex-col gap-2 py-4 transition-all duration-500",
        isHighlighted && "rounded-md ring-2 ring-primary/40",
      )}
      data-block-id={block.id}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      {/* User message */}
      <div className="flex flex-col gap-1 rounded-md bg-primary/10 px-3 py-2">
        <Badge variant="secondary" className="self-start">
          You
        </Badge>
        {block.attachments.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {block.attachments.map((a) => (
              <Badge key={a.id} variant="outline" className="gap-1 font-normal text-muted-foreground">
                {a.kind === "comment" ? (
                  <MessageSquare className="size-3" />
                ) : a.kind === "image" ? (
                  <ImageIcon className="size-3" />
                ) : (
                  <Paperclip className="size-3" />
                )}
                {a.label}
              </Badge>
            ))}
          </div>
        )}
        <div className="whitespace-pre-wrap break-words text-sm text-foreground">
          <UserMessageContent
            text={block.userContent}
            onPillClick={(label) => setActivePillLabel((prev) => (prev === label ? null : label))}
          />
        </div>
      </div>

      {/* Prompt pill popover (read-only) — shown when a /slug pill is clicked */}
      {activePillPrompt && (
        <div className="relative">
          <PromptPopover
            prompt={{
              id: activePillPrompt.id,
              label: activePillPrompt.label,
              content: activePillPrompt.content ?? "",
            }}
            onClose={() => setActivePillLabel(null)}
          />
        </div>
      )}

      {/* Content stream — renders text, tool cards, and thinking in order */}
      {(block.contentStream.length > 0 || block.status === "running") && (
        <div className="flex flex-col gap-1 px-1">
          <Badge variant="outline" className="self-start font-normal text-muted-foreground">
            {block.agentName || "Assistant"}
          </Badge>
          <ContentStreamView segments={block.contentStream} isStreaming={isStreaming} />
        </div>
      )}

      {/* Live tasks — active tool calls while this block is streaming */}
      <LiveTasksView traceEntries={block.traceEntries} isStreaming={isStreaming} />

      {/* Status error */}
      {block.status === "fail" && block.contentStream.length === 0 && (
        <div className="text-xs italic text-destructive">(run failed)</div>
      )}

      {/* Trace — shown below the content stream (position unchanged) */}
      <TraceViewer entries={block.traceEntries} startExpanded={isStreaming} />

      {/* Comment annotations */}
      {pinnedComments.map((c) => (
        <div
          key={c.id}
          data-comment-id={c.id}
          className="flex items-center gap-1.5 rounded border-l-2 border-primary bg-primary/10 px-2 py-1 text-xs text-primary"
        >
          <MessageSquare className="size-3 shrink-0" />
          <span className="truncate">"{c.selectedText.slice(0, 80)}"</span>
        </div>
      ))}

      {/* Thread */}
      {block.thread && (
        <ThreadSummary
          thread={block.thread}
          onExpand={() => {
            /* noop */
          }}
        />
      )}

      {/* Checkpoint controls — appear on hover for completed blocks */}
      {!isStreaming && hovered && (onRestore || onFork) && (
        <div className="flex items-center gap-1.5 pt-0.5">
          {onRestore && (
            <Tip tip="Restore chat to this checkpoint (discards subsequent blocks)">
              <Button variant="outline" size="xs" onClick={() => onRestore(block.id)}>
                <Undo2 data-icon="inline-start" />
                Restore
              </Button>
            </Tip>
          )}
          {onFork && (
            <Tip tip="Fork a new chat from this checkpoint">
              <Button variant="outline" size="xs" onClick={() => onFork(block.id)}>
                <GitFork data-icon="inline-start" />
                Fork
              </Button>
            </Tip>
          )}
        </div>
      )}
    </div>
  );
}

function ShellBlockView({
  block,
  onAction,
  isHighlighted,
}: {
  block: ShellBlock;
  onAction: (action: string, b: ShellBlock) => void;
  isHighlighted?: boolean;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const duration = block.endedAt && block.startedAt ? fmtDuration(block.endedAt - block.startedAt) : null;

  const StatusIcon = block.status === "ok" ? Check : block.status === "fail" ? X : Loader2;
  const statusIconClass = cn(
    "size-3.5 shrink-0",
    block.status === "ok" && "text-primary",
    block.status === "fail" && "text-destructive",
    block.status === "running" && "animate-spin text-amber-500",
  );

  return (
    <div
      className={cn(
        "flex flex-col gap-1.5 py-4 transition-all duration-500",
        isHighlighted && "rounded-md ring-2 ring-primary/40",
      )}
      data-block-id={block.id}
    >
      {/* Header — highlighted command prompt */}
      <div className="flex items-center gap-2 rounded-md bg-primary/10 px-3 py-1.5">
        <StatusIcon className={statusIconClass} />
        <span className="flex-1 font-mono text-sm text-foreground">$ {block.command}</span>
        {block.exitCode !== null && block.exitCode !== 0 && (
          <span className="text-xs text-destructive">exit {block.exitCode}</span>
        )}
        {duration && <span className="text-xs text-muted-foreground">{duration}</span>}
        <Button
          variant="ghost"
          size="icon-xs"
          onClick={() => setCollapsed((c) => !c)}
          aria-label={collapsed ? "Expand" : "Collapse"}
        >
          {collapsed ? <ChevronRight /> : <ChevronDown />}
        </Button>
      </div>

      {/* Output */}
      {!collapsed && block.output && (
        <pre className="max-h-80 overflow-y-auto rounded bg-background px-3 py-1.5 font-mono text-xs text-muted-foreground">
          {block.output}
        </pre>
      )}

      {/* Action bar */}
      {block.status !== "running" && (
        <div className="flex gap-2 pt-0.5">
          {(["Explain", "Fix", "Retry"] as const).map((act) => (
            <Button key={act} variant="outline" size="xs" onClick={() => onAction(act.toLowerCase(), block)}>
              {act}
            </Button>
          ))}
        </div>
      )}

      {/* Comment annotations */}
      {block.comments
        .filter((c) => c.pinnedToPrompt)
        .map((c) => (
          <div
            key={c.id}
            data-comment-id={c.id}
            className="flex items-center gap-1.5 rounded border-l-2 border-primary bg-primary/10 px-2 py-1 text-xs text-primary"
          >
            <MessageSquare className="size-3 shrink-0" />
            <span className="truncate">"{c.selectedText.slice(0, 80)}"</span>
          </div>
        ))}

      {/* Thread */}
      {block.thread && (
        <ThreadSummary
          thread={block.thread}
          onExpand={() => {
            /* noop */
          }}
        />
      )}
    </div>
  );
}

function BlockView({
  block,
  isStreaming,
  onShellAction,
  isHighlighted,
  workspacePrompts,
  onRestoreBlock,
  onForkBlock,
}: {
  block: Block;
  isStreaming: boolean;
  onShellAction: (action: string, b: ShellBlock) => void;
  isHighlighted?: boolean;
  workspacePrompts?: PickerItem[];
  onRestoreBlock?: (blockId: string) => void;
  onForkBlock?: (blockId: string) => void;
}) {
  if (block.kind === "conversation") {
    return (
      <ConversationBlockView
        block={block}
        isStreaming={isStreaming}
        isHighlighted={isHighlighted}
        workspacePrompts={workspacePrompts}
        onRestore={onRestoreBlock}
        onFork={onForkBlock}
      />
    );
  }
  return <ShellBlockView block={block} onAction={onShellAction} isHighlighted={isHighlighted} />;
}

// ── Attachment tray ─────────────────────────────────────────────────────

function AttachmentTray({
  attachments,
  onRemove,
  onCommentClick,
}: {
  attachments: Attachment[];
  onRemove: (id: string) => void;
  onCommentClick?: (a: Attachment) => void;
}) {
  if (attachments.length === 0) return null;
  const comments = attachments.filter((a) => a.kind === "comment");
  const files = attachments.filter((a) => a.kind === "file");
  const images = attachments.filter((a) => a.kind === "image");

  const Pill = ({ a }: { a: Attachment }) => {
    const Icon = a.kind === "comment" ? MessageSquare : a.kind === "image" ? ImageIcon : Paperclip;
    const clickable = a.kind === "comment" && onCommentClick;
    return (
      <Badge variant="outline" className="gap-1 font-normal text-muted-foreground">
        <Icon className="size-3" />
        <span
          className={cn("max-w-[100px] truncate", clickable && "cursor-pointer hover:text-foreground")}
          onClick={clickable ? () => onCommentClick(a) : undefined}
        >
          {a.label}
        </span>
        <Button
          variant="ghost"
          size="icon-xs"
          onClick={() => onRemove(a.id)}
          className="-mr-1 size-4 text-muted-foreground hover:text-destructive"
          aria-label="Remove attachment"
        >
          <X />
        </Button>
      </Badge>
    );
  };

  return (
    <div className="flex flex-wrap gap-1.5 border-t border-border px-2 py-1.5">
      {comments.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {comments.map((a) => (
            <Pill key={a.id} a={a} />
          ))}
        </div>
      )}
      {files.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {files.map((a) => (
            <Pill key={a.id} a={a} />
          ))}
        </div>
      )}
      {images.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {images.map((a) => (
            <Pill key={a.id} a={a} />
          ))}
        </div>
      )}
    </div>
  );
}

// ── Main App component ─────────────────────────────────────────────────

export function App() {
  const [state, dispatch] = useStore();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const timelineRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [agentLoadError, setAgentLoadError] = useState<string | null>(null);
  const [inputMode, setInputMode] = useState<"chat" | "shell" | "command">("chat");
  const [commentDraft, setCommentDraft] = useState("");
  /** Per-chat reasoning_effort override. "" = use agent/provider default. */
  const [reasoningEffort, setReasoningEffortState] = useState<ReasoningEffort>(() => loadReasoningEffort());
  const reasoningEffortRef = useRef(reasoningEffort);
  reasoningEffortRef.current = reasoningEffort;
  /** Per-chat Anthropic adaptive-effort override. "" = use server default. */
  const [anthropicEffort, setAnthropicEffortState] = useState<AnthropicEffort>(() => loadAnthropicEffort());
  const anthropicEffortRef = useRef(anthropicEffort);
  anthropicEffortRef.current = anthropicEffort;

  // ── slash / @ picker state ─────────────────────────────────────────────
  const [picker, setPicker] = useState<PickerState | null>(null);
  const [pickerIdx, setPickerIdx] = useState(0);
  const [workspacePrompts, setWorkspacePrompts] = useState<PickerItem[]>([]);
  const [workspaceRoot, setWorkspaceRoot] = useState("");
  /** Model options grouped by provider name, loaded from llm.providers.get.
   * Carries each group's provider config so picking a model from a non-active
   * group can override the request's base_url / api_key / kind for that run. */
  const [modelGroups, setModelGroups] = useState<
    {
      label: string;
      id: string;
      kind: string;
      base_url: string;
      api_key: string;
      models: string[];
    }[]
  >([]);
  /** ID + kind of the currently-active provider — used as the fallback when
   * `state.model` is empty (provider default) or doesn't match any known
   * model entry, and to detect when the picked model belongs to a non-active
   * group (triggering per-run provider override). */
  const [activeProviderId, setActiveProviderId] = useState<string>("");
  const [activeProviderKind, setActiveProviderKind] = useState<string>("");
  /** Controls the model Combobox popover */
  const [modelComboOpen, setModelComboOpen] = useState(false);
  /** Prompt pills attached to the current message (like VS Code slash commands). */
  const [attachedPrompts, setAttachedPrompts] = useState<{ id: string; label: string; content: string }[]>([]);
  /** ID of the pill whose PromptPopover is currently open (null = none). */
  const [activePillId, setActivePillId] = useState<string | null>(null);

  /** Session-scoped global approval mode: overrides per-category trust for this browser tab. */
  const [globalApprovalMode, setGlobalApprovalMode] = useState<"default" | "autopilot" | "bypass">(() => {
    try {
      const stored = sessionStorage.getItem("cronymax.global_approval_mode");
      if (stored === "autopilot" || stored === "bypass") return stored;
    } catch {
      /* ignore */
    }
    return "default";
  });

  function persistGlobalApprovalMode(mode: "default" | "autopilot" | "bypass") {
    try {
      if (mode === "default") {
        sessionStorage.removeItem("cronymax.global_approval_mode");
      } else {
        sessionStorage.setItem("cronymax.global_approval_mode", mode);
      }
    } catch {
      /* ignore */
    }
  }

  // Load workspace prompts + root + provider models on mount.

  /** Latest token usage from the most recent assistant_turn across all blocks. */
  const latestUsage = useMemo(() => {
    const blocks = state.blocks;
    for (let i = blocks.length - 1; i >= 0; i--) {
      const blk = blocks[i];
      if (!blk || blk.kind !== "conversation") continue;
      const traces = (blk as import("./store").ConversationBlock).traceEntries;
      for (let j = traces.length - 1; j >= 0; j--) {
        const t = traces[j];
        if (t && t.kind === "assistant_turn" && t.usage) return t.usage;
      }
    }
    return null;
  }, [state.blocks]);

  /** Context limit in tokens for the active model (null = unknown). */
  const contextLimit = useMemo(() => resolveContextLimit(state.model), [state.model]);

  /** All file changes aggregated across the entire session's conversation blocks. */
  const sessionFileChanges = useMemo(() => {
    const changes: FileChange[] = [];
    for (const blk of state.blocks) {
      if (blk.kind === "conversation" && blk.fileChanges) {
        changes.push(...blk.fileChanges);
      }
    }
    return changes;
  }, [state.blocks]);

  useEffect(() => {
    shells.browser.workspace.prompts
      .list()
      .then((res) => {
        setWorkspacePrompts(
          res.prompts.map((p) => {
            const { description, content } = parseFrontmatter(p.content);
            return {
              id: `ws-${p.name}`,
              label: p.name,
              description: description ?? content.slice(0, 70).replace(/\n/g, " ").trim(),
              content,
            };
          }),
        );
      })
      .catch(() => undefined);
    shells.browser.space
      .list()
      .then((spaces) => {
        const active = spaces.find((s) => s.active);
        if (active) setWorkspaceRoot(active.root_path);
      })
      .catch(() => undefined);
    // Load model list from configured providers
    shells.browser.llm.providers
      .get()
      .then(async ({ raw, active_id }) => {
        if (!raw) return;
        interface StoredProvider {
          id: string;
          name: string;
          kind: "openai" | "anthropic" | "ollama" | "github_copilot" | "custom";
          base_url: string;
          api_key: string;
          default_model: string;
        }
        const providers: StoredProvider[] = JSON.parse(raw);
        const active = providers.find((p) => p.id === active_id);
        if (active) {
          setActiveProviderId(active.id);
          setActiveProviderKind(active.kind);
        }
        const groups: {
          label: string;
          id: string;
          kind: string;
          base_url: string;
          api_key: string;
          models: string[];
        }[] = [];
        for (const p of providers) {
          if (!p.base_url) continue;
          let models: string[] = [];
          try {
            models = await listProviderModels(p);
          } catch {
            /* keep models empty; fall through to default_model below */
          }
          if (models.length === 0 && p.default_model) models = [p.default_model];
          if (models.length > 0)
            groups.push({
              label: p.name || p.kind,
              id: p.id,
              kind: p.kind,
              base_url: p.base_url,
              api_key: p.api_key,
              models,
            });
        }
        setModelGroups(groups);
      })
      .catch(() => undefined);
  }, []);

  // Selection tooltip — freeze when comment input is focused so it doesn't
  // disappear when the browser clears the selection on input focus.
  const selectionInfo = useSelectionTooltip(timelineRef);
  const [frozenSelection, setFrozenSelection] = useState<import("./useSelectionTooltip").SelectionInfo | null>(null);
  const activeSelection = frozenSelection ?? selectionInfo;

  // Reset the comment draft whenever the user starts a *new* selection.
  // We compare against the last seen (blockId, selectedText) identity rather
  // than against `activeSelection` because focusing the textarea makes the
  // browser clear its selection (selectionInfo → null) while we hold the
  // tooltip open via `frozenSelection`; clearing the draft on that transient
  // null would wipe what the user is typing.
  const lastSelectionKeyRef = useRef<string | null>(null);
  useEffect(() => {
    if (!selectionInfo) return; // frozen or no selection — keep draft
    const key = `${selectionInfo.blockId}::${selectionInfo.selectedText}`;
    if (key === lastSelectionKeyRef.current) return; // same selection — keep draft
    lastSelectionKeyRef.current = key;
    setCommentDraft("");
  }, [selectionInfo]);

  // ── comment attachment → scroll & highlight ────────────────────────────
  const [highlightedBlockId, setHighlightedBlockId] = useState<string | null>(null);
  const onCommentAttachmentClick = (a: Attachment) => {
    if (!a.commentId) return;
    // Find the block that owns this comment
    let blockId: string | undefined;
    for (const blk of state.blocks) {
      if (blk.comments.find((c) => c.id === a.commentId)) {
        blockId = blk.id;
        break;
      }
    }
    if (!blockId) return;
    // Scroll to the comment annotation, or the block if annotation not rendered yet
    const target =
      timelineRef.current?.querySelector(`[data-comment-id="${a.commentId}"]`) ??
      timelineRef.current?.querySelector(`[data-block-id="${blockId}"]`);
    target?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    setHighlightedBlockId(blockId);
    setTimeout(() => setHighlightedBlockId(null), 1600);
  };

  // ── agent catalog ─────────────────────────────────────────────────────
  const refreshAgents = async () => {
    try {
      const res = await agentRegistry.list();
      dispatch({ type: "setAgents", agents: res.agents ?? [] });
      setAgentLoadError(null);
    } catch (err) {
      setAgentLoadError((err as Error).message);
    }
  };

  // ── ensure terminal session for this chat tab ─────────────────────────
  const ensureChatTerminal = async (currentTid: string | null, chatId: string) => {
    // Validate the cached terminal ID: it won't survive an app restart,
    // so check whether it's still present in the C++ process before reusing.
    if (currentTid) {
      try {
        const { items } = await shells.browser.terminal.list();
        if (items.some((t) => t.id === currentTid)) return currentTid;
      } catch {
        // Fall through to create a new terminal.
      }
    }
    try {
      const newTid = await shells.browser.terminal.new();
      const tid = typeof newTid === "string" ? newTid : (newTid as { id: string }).id;
      await rt_terminal.start(tid);
      dispatch({ type: "setTerminalTid", tid });
      // Persist immediately
      const { data } = loadChatData(chatId);
      persistChatData(chatId, { ...data, terminalTid: tid });
      return tid;
    } catch {
      return null;
    }
  };

  // ── init ────────────────────────────────────────────────────────────────
  useEffect(() => {
    const init = async () => {
      // Ask the native shell which tab we are, so we can restore the same
      // chatId that was bound to this tab in a previous session.
      try {
        const tabInfo = await shells.browser.shell.this_tab_id();
        if (tabInfo?.meta?.chat_id) {
          // Seed sessionStorage so ensureChat() picks up the persisted chatId.
          sessionStorage.setItem("cronymax_chat_tab_id", tabInfo.meta.chat_id);
        }
      } catch {
        // Bridge may not be ready on first run or in dev; fall through.
      }

      const { id, name } = ensureChat();

      // Register this tab's chatId with the native shell so it survives
      // the next app restart (no-op if already registered with same value).
      void shells.browser.shell.tab_set_meta({ key: "chat_id", value: id }).catch(() => {
        /* ignore */
      });

      const { data, migrationNotice } = loadChatData(id);
      const model = data.model || loadSelectedModel();
      dispatch({
        type: "loadChat",
        id,
        name,
        blocks: data.blocks,
        terminalTid: data.terminalTid,
        model,
        agentId: data.agentId,
        migrationNotice,
      });
      const { flows, selected } = loadFlowsList();
      dispatch({ type: "setFlows", flows, selected });
      void refreshAgents();
      void ensureChatTerminal(data.terminalTid, id);
    };

    void init();

    const onStorage = (e: StorageEvent) => {
      if (e.key === "flows" || e.key === "active_flow") {
        const refreshed = loadFlowsList();
        dispatch({
          type: "setFlows",
          flows: refreshed.flows,
          selected: refreshed.selected,
        });
      }
      if (e.key === "chats") {
        if (state.activeChatId) {
          const { data: d, migrationNotice: mn } = loadChatData(state.activeChatId);
          dispatch({
            type: "loadChat",
            id: state.activeChatId,
            name: chatNameFor(state.activeChatId),
            blocks: d.blocks,
            terminalTid: d.terminalTid,
            model: d.model || loadSelectedModel(),
            agentId: d.agentId,
            migrationNotice: mn,
          });
        }
      }
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
    // Run once on mount. refreshAgents/ensureChatTerminal are plain functions
    // (re-created each render); listing them here would re-fire init every
    // render and reset state.blocks via loadChat, making just-sent message
    // cards flash and disappear.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Use refs so the listener always reads latest values without re-subscribing.
  const runningBlockIdRef = useRef<string | null>(null);
  runningBlockIdRef.current = state.runningBlockId;

  // ── terminal output → ShellBlock accumulation ──────────────────────────
  // Topic-scoped to the active terminal; auto-resubscribes on space switch.
  useRuntimeEvent(state.terminalTid ? `terminal:${state.terminalTid}` : "", (event: unknown) => {
    const blockId = runningBlockIdRef.current;
    if (!blockId) return;

    const info = shellSentinelRef.current.get(blockId);
    if (!info) return;

    let data: string;
    try {
      const ev = event as Record<string, unknown>;
      const pl = ev?.payload as Record<string, unknown> | undefined;
      if (pl?.kind !== "raw") return;
      const dataObj = pl?.data as Record<string, unknown> | undefined;
      const b64 = dataObj?.data as string | undefined;
      if (!b64) return;
      data = b64ToUtf8(b64);
    } catch {
      return;
    }

    // Phase 1: waiting for start marker — discard all preamble
    // (shell prompts, echoed command line, etc.)
    if (!info.capturing) {
      const startIdx = data.indexOf(info.start);
      if (startIdx === -1) return; // still preamble — discard chunk
      // Skip to the line after the start marker line
      const nl = data.indexOf("\n", startIdx);
      data = nl !== -1 ? data.slice(nl + 1) : "";
      info.capturing = true;
      if (!data) return;
    }

    // Phase 2: capturing — look for end marker
    const endRe = new RegExp(`${info.end}:(\\d+)`);
    const match = endRe.exec(data);
    if (match) {
      const exitCode = parseInt(match[1]!, 10);
      // Deliver output before the end-marker line (strip trailing partial line)
      const cleanData = data.slice(0, match.index).replace(/[^\n]*$/, "");
      if (cleanData) {
        dispatch({
          type: "appendShellOutput",
          id: blockId,
          chunk: cleanData,
          now: Date.now(),
        });
      }
      dispatch({
        type: "finalizeShellBlock",
        id: blockId,
        exitCode,
        now: Date.now(),
      });
      dispatch({ type: "setRunning", running: false });
      dispatch({ type: "setRunningBlockId", id: null });
      runningBlockIdRef.current = null;
      shellSentinelRef.current.delete(blockId);
      return;
    }

    dispatch({
      type: "appendShellOutput",
      id: blockId,
      chunk: data,
      now: Date.now(),
    });
  });

  // ── auto scroll ──────────────────────────────────────────────────────
  // Use useEffect (async, after paint) instead of useLayoutEffect so we never
  // force a synchronous layout read (el.scrollHeight) while Blink is still in
  // its layout phase — that triggers the DisplayLock DCHECK crash with large
  // histories.  Also only scroll when the user is already near the bottom so
  // we don't hijack the scroll position when reviewing older messages.
  useEffect(() => {
    const raf = requestAnimationFrame(() => {
      const el = timelineRef.current;
      if (!el) return;
      const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
      // Treat within 120 px of the bottom (or actively streaming) as "at bottom".
      if (distanceFromBottom <= 120 || state.running) {
        el.scrollTop = el.scrollHeight;
      }
    });
    return () => cancelAnimationFrame(raf);
  }, [state.blocks, state.running]);

  // ── input mode detection + prefix auto-strip ─────────────────────────
  const onInputChange = (e: ChangeEvent<HTMLTextAreaElement>) => {
    const el = e.currentTarget;
    const v = el.value;
    const caret = el.selectionStart ?? v.length;

    // Shell mode: text starts with "$"
    if (v.startsWith("$")) {
      setInputMode("shell");
      el.value = v.startsWith("$ ") ? v.slice(2) : v.slice(1);
      setPicker(null);
      return;
    }

    if (v === "") {
      setInputMode("chat");
      setPicker(null);
      return;
    }

    // Look for the trigger character that starts the current "word" at cursor.
    // We search backwards from the caret to find the nearest trigger.
    const textBeforeCaret = v.slice(0, caret);

    // Find `/` trigger: only at start of a line or the very beginning of input.
    const slashMatch = textBeforeCaret.match(/(?:^|\n)(\/[^\s]*)$/);
    if (slashMatch) {
      const query = slashMatch[1]!.slice(1); // strip the leading "/"
      const triggerStart = caret - slashMatch[1]!.length;
      setPicker({ type: "slash", query, triggerStart });
      setPickerIdx(0);
      setInputMode("command");
      return;
    }

    // Find `@` trigger: at word boundary anywhere in the input.
    const atMatch = textBeforeCaret.match(/(?:^|[\s,]|^)(@[^\s@]*)$/);
    if (atMatch) {
      const query = atMatch[1]!.slice(1); // strip the leading "@"
      const triggerStart = caret - atMatch[1]!.length;
      setPicker({ type: "at", query, triggerStart });
      setPickerIdx(0);
      if (inputMode !== "shell") setInputMode("chat");
      return;
    }

    // No active trigger — close picker and update mode normally
    setPicker(null);
    if (inputMode === "command") setInputMode("chat");
  };

  // ── paste → attach ────────────────────────────────────────────────────
  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const items = Array.from(e.clipboardData.items);
    const imageItem = items.find((i) => i.type.startsWith("image/"));
    if (imageItem) {
      e.preventDefault();
      const file = imageItem.getAsFile();
      if (!file) return;
      const reader = new FileReader();
      reader.onload = (ev) => {
        const dataUrl = ev.target?.result as string;
        dispatch({
          type: "addAttachment",
          attachment: {
            id: crypto.randomUUID(),
            kind: "image",
            label: file.name || "pasted-image",
            content: dataUrl,
          },
        });
      };
      reader.readAsDataURL(file);
    }
  };

  // ── file picker ────────────────────────────────────────────────────────
  const onFileChange = (e: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? []);
    for (const file of files) {
      const reader = new FileReader();
      reader.onload = (ev) => {
        const content = ev.target?.result as string;
        dispatch({
          type: "addAttachment",
          attachment: {
            id: crypto.randomUUID(),
            kind: file.type.startsWith("image/") ? "image" : "file",
            label: file.name,
            content,
          },
        });
      };
      if (file.type.startsWith("image/")) {
        reader.readAsDataURL(file);
      } else {
        reader.readAsText(file);
      }
    }
    e.target.value = "";
  };

  // Per-block sentinel info: start marker, end marker, and whether the start
  // marker has already been seen (so preamble / echo lines are discarded).
  const shellSentinelRef = useRef<Map<string, { start: string; end: string; capturing: boolean }>>(new Map());

  // ── send / run ─────────────────────────────────────────────────────────
  const onRun = async (rawText: string, displayText?: string) => {
    if (state.running || state.isReconnecting || !state.activeChatId) return;
    const chatId = state.activeChatId;

    // Detect shell mode: rawText starts with "$" OR inputMode is "shell"
    const isShellCmd = rawText.trimStart().startsWith("$") || inputMode === "shell";
    if (isShellCmd) {
      // Strip leading "$ " or "$" if present (user may have typed it or mode stripped it)
      const command = rawText.replace(/^\s*\$\s*/, "").trim();
      const tid = await ensureChatTerminal(state.terminalTid, chatId);
      if (!tid) return;

      const blockId = crypto.randomUUID();
      // Start/end sentinels — only hex chars, safe through JSON serialization.
      // Anything before the start marker (shell prompt, command echo) is discarded;
      // only what appears between start and end is shown as output.
      const nonce = Array.from(crypto.getRandomValues(new Uint8Array(8)))
        .map((b) => b.toString(16).padStart(2, "0"))
        .join("");
      const startMarker = `__cx_s_${nonce}__`;
      const endMarker = `__cx_e_${nonce}__`;
      shellSentinelRef.current.set(blockId, {
        start: startMarker,
        end: endMarker,
        capturing: false,
      });

      const shellBlock: import("./store").ShellBlock = {
        kind: "shell",
        id: blockId,
        command,
        output: "",
        rawBuf: "",
        status: "running",
        exitCode: null,
        startedAt: Date.now(),
        endedAt: null,
        comments: [],
      };
      dispatch({ type: "createBlock", block: shellBlock });
      dispatch({ type: "setRunningBlockId", id: blockId });
      dispatch({ type: "setRunning", running: true });
      dispatch({ type: "clearPinnedComments" });
      // Eagerly update the block-id ref so the runtime event handler is
      // ready immediately — before React re-renders and updates from state.
      runningBlockIdRef.current = blockId;

      // Bracket the command with start/end markers so preamble (prompt, echo)
      // is automatically discarded. Single-quoted start (no expansion needed).
      const wrapped = `echo '${startMarker}'; ${command}; _ec=$?; echo "${endMarker}:$_ec"`;
      try {
        await rt_terminal.run(tid, wrapped);
      } catch {
        dispatch({
          type: "finalizeShellBlock",
          id: blockId,
          exitCode: 1,
          now: Date.now(),
        });
        dispatch({ type: "setRunning", running: false });
        dispatch({ type: "setRunningBlockId", id: null });
        shellSentinelRef.current.delete(blockId);
        return;
      }

      // Safety timeout 5 min
      setTimeout(
        () => {
          if (runningBlockIdRef.current === blockId) {
            dispatch({ type: "setRunning", running: false });
            dispatch({ type: "setRunningBlockId", id: null });
            shellSentinelRef.current.delete(blockId);
          }
        },
        5 * 60 * 1000,
      );
      return;
    }

    // ── Normal conversation run ─────────────────────────────────────
    dispatch({ type: "setRunning", running: true });

    let speaker = "";
    let body = rawText;
    const agentNames = state.agents.map((a) => a.name);
    const parsed = parseMention(rawText, agentNames);
    if (parsed.agent) {
      speaker = parsed.agent;
      body = parsed.body;
    } else if (state.selectedFlow) {
      speaker = state.selectedFlow;
    } else {
      speaker = agentNames[0] || "";
    }

    const blockId = crypto.randomUUID();
    const block: import("./store").ConversationBlock = {
      kind: "conversation",
      id: blockId,
      userContent: displayText ?? rawText,
      attachments: state.attachments.slice(),
      contentStream: [],
      assistantContent: "",
      agentName: speaker || undefined,
      traceEntries: [],
      fileChanges: [],
      status: "running",
      comments: [],
      createdAt: Date.now(),
      thinkingText: "",
      thinkingSealed: false,
      thinkingStartedAt: null,
      thinkingElapsedMs: 0,
    };
    dispatch({ type: "createBlock", block });
    dispatch({ type: "setRunningBlockId", id: blockId });
    // Clear attachments from tray after capturing snapshot into block
    dispatch({ type: "clearAttachments" });
    dispatch({ type: "clearPinnedComments" });

    let hasContent = false;
    let thinkingStartedAt: number | null = null;
    let thinkingSealed = false;
    // Dedup key = "<subscription_id>:<sequence>" so that concurrent
    // conversations whose Rust subscriptions independently start at seq=0
    // never cross-contaminate each other's dedup sets.
    const seenSeqs = new Set<string>();
    let runId = "";
    // Last error surfaced via a "trace.error" event from the runtime
    // (e.g. LLM HTTP failure). Used as the assistantText fallback when
    // the run terminates without producing any tokens.
    let lastErrorMessage = "";

    // Track pending review info from awaiting_review status so we can
    // pair it with the arriving PermissionRequest event.
    let pendingReviewId: string | null = null;

    const off = browser.on("event", (raw: unknown) => {
      const ev = raw as Record<string, unknown> | null;
      if (!ev) return;

      if (ev.tag === "event") {
        const inner = (ev.event as Record<string, unknown> | undefined) ?? {};
        const pl = (inner.payload as Record<string, unknown> | undefined) ?? {};
        const pRunId =
          (pl.run_id as string | undefined) ?? ((inner as Record<string, unknown>).run_id as string | undefined);
        // Filter by run_id BEFORE deduplicating so that a different run's
        // seq=0 does not consume our seq=0 from the dedup set.
        if (pRunId && runId && pRunId !== runId) return;
        const subId = (ev.subscription as string | undefined) ?? "";
        const seq = inner.sequence as number | undefined;
        if (typeof seq === "number") {
          const dedupKey = `${subId}:${seq}`;
          if (seenSeqs.has(dedupKey)) return;
          seenSeqs.add(dedupKey);
        }
        const kind = pl.kind as string | undefined;

        if (kind === "thinking_token") {
          const delta = pl.delta as string | undefined;
          if (delta) {
            if (thinkingStartedAt === null) {
              thinkingStartedAt = Date.now();
            }
            dispatch({
              type: "appendThinkingSegment",
              id: blockId,
              delta,
            });
          }
        } else if (kind === "token") {
          // First text token seals the thinking segment if thinking is in progress.
          if (thinkingStartedAt !== null && !thinkingSealed) {
            thinkingSealed = true;
            const elapsedMs = Date.now() - thinkingStartedAt;
            dispatch({ type: "sealThinkingSegment", id: blockId, elapsedMs });
          }
          const content = (pl.delta ?? pl.content) as string | undefined;
          if (content) {
            hasContent = true;
            dispatch({
              type: "appendContentText",
              id: blockId,
              delta: content,
            });
          }
        } else if (kind === "run_status") {
          const status = pl.status as string | undefined;
          if (status === "succeeded" || status === "failed" || status === "cancelled") {
            dispatch({ type: "clearAwaitingApproval" });
            // Seal any pending thinking segment before finalizing.
            if (thinkingStartedAt !== null && !thinkingSealed) {
              thinkingSealed = true;
              const elapsedMs = Date.now() - thinkingStartedAt;
              dispatch({ type: "sealThinkingSegment", id: blockId, elapsedMs });
            }
            if (!hasContent) {
              // Prefer a concrete failure reason if the runtime surfaced
              // one (RunStatus.detail.message or a prior trace.error).
              // Fall back to the generic placeholder.
              const detail = (pl.detail as Record<string, unknown> | undefined) ?? {};
              const detailMsg = typeof detail.message === "string" ? detail.message : "";
              let fallback: string;
              if (status === "succeeded") {
                fallback = "(completed)";
              } else if (detailMsg) {
                fallback = `(${status}) ${detailMsg}`;
              } else if (lastErrorMessage) {
                fallback = `(${status}) ${lastErrorMessage}`;
              } else {
                fallback = "(no output)";
              }
              dispatch({
                type: "appendContentText",
                id: blockId,
                delta: fallback,
              });
              hasContent = true;
            }
            dispatch({
              type: "finalizeBlock",
              id: blockId,
              status: status === "succeeded" ? "ok" : "fail",
              agentName: speaker || undefined,
            });

            // Persist
            const { data } = loadChatData(chatId);
            persistChatData(chatId, { ...data, blocks: [...data.blocks] });

            off();
            dispatch({ type: "setCurrentRunId", runId: null });
            dispatch({ type: "setRunning", running: false });
            dispatch({ type: "setRunningBlockId", id: null });
            inputRef.current?.focus();
          } else if (status === "awaiting_review") {
            // Agent is waiting for approval — store the review_id if available
            const rid = pl.review_id as string | undefined;
            if (rid) pendingReviewId = rid;
            // Don't finalize or stop running — just await PermissionRequest event
          } else if (status === "running") {
            // Resumed after approval was granted
            dispatch({ type: "clearAwaitingApproval" });
          }
        } else if (kind === "permission_request") {
          // Tool approval request: check global mode first, then per-category trust
          const reviewId = (pl.review_id as string | undefined) ?? pendingReviewId ?? "";
          const req = (pl.request as Record<string, unknown> | undefined) ?? {};
          const toolName = (req.tool_name as string | undefined) ?? (pl.tool_name as string | undefined) ?? "";
          const args = req.args ?? pl.args ?? {};
          const category = toolName.split("_")[0] ?? toolName;

          // Global approval mode takes precedence over per-category trust map
          let effectiveTrust: "autopilot" | "bypass" | "ask";
          if (globalApprovalMode === "autopilot") {
            effectiveTrust = "autopilot";
          } else if (globalApprovalMode === "bypass") {
            effectiveTrust = "bypass";
          } else {
            // Read per-category trust level from localStorage
            const trustMap = loadTrustMap();
            effectiveTrust = (trustMap[category] ?? "ask") as "autopilot" | "bypass" | "ask";
          }

          if (effectiveTrust === "autopilot") {
            browser.send("review.approve", { review_id: reviewId }).catch(() => undefined);
          } else if (effectiveTrust === "bypass") {
            browser.send("review.request_changes", { review_id: reviewId }).catch(() => undefined);
          } else {
            // "ask" — show the approval card
            dispatch({
              type: "setAwaitingApproval",
              runId,
              reviewId,
              toolName,
              args,
            });
            dispatch({
              type: "appendTraceEntry",
              id: blockId,
              entry: {
                kind: "approval_request",
                reviewId,
                tool: toolName,
                args,
                ts: Date.now(),
              },
            });
          }
        } else if (kind === "trace") {
          // Structured trace events from the runtime
          const trace = (pl.trace as Record<string, unknown> | undefined) ?? pl;
          const traceKind = trace.kind as string | undefined;

          if (traceKind === "run_start") {
            dispatch({
              type: "appendTraceEntry",
              id: blockId,
              entry: {
                kind: "run_start",
                model: (trace.model as string | undefined) ?? "",
                systemPrompt: (trace.system_prompt as string | undefined) ?? "",
                userInput: (trace.user_input as string | undefined) ?? "",
                tools: (trace.tools as string[] | undefined) ?? [],
                turnsLimit: (trace.turns_limit as number | undefined) ?? 0,
                ts: Date.now(),
              },
            });
          } else if (traceKind === "assistant_turn") {
            // Extract optional usage and duration emitted by agent-run-middleware
            const usageRaw = trace.usage as Record<string, number> | undefined;
            const usage = usageRaw
              ? {
                  inputTokens: (usageRaw.input_tokens as number) ?? 0,
                  outputTokens: (usageRaw.output_tokens as number) ?? 0,
                }
              : undefined;
            const turnDurationMs = trace.duration_ms as number | undefined;
            dispatch({
              type: "appendTraceEntry",
              id: blockId,
              entry: {
                kind: "assistant_turn",
                // Rust emits "turn" (not "turn_id")
                turnId: (trace.turn as number | undefined) ?? (trace.turn_id as number | undefined) ?? 0,
                text: (trace.text as string | undefined) ?? "",
                finishReason: (trace.finish_reason as string | undefined) ?? "",
                ts: Date.now(),
                ...(usage ? { usage } : {}),
                ...(turnDurationMs != null ? { durationMs: turnDurationMs } : {}),
              },
            });
          } else if (traceKind === "tool_start") {
            const toolCallId = (trace.tool_call_id as string | undefined) ?? "";
            const tool = (trace.tool as string | undefined) ?? "";
            const args = trace.arguments ?? trace.args ?? {};
            dispatch({
              type: "appendTraceEntry",
              id: blockId,
              entry: {
                kind: "tool_start",
                toolCallId,
                tool,
                args,
                ts: Date.now(),
              },
            });
            // Also add a running tool_call segment to the content stream
            dispatch({
              type: "appendToolCallSegment",
              id: blockId,
              toolCallId,
              tool,
              args,
            });
          } else if (traceKind === "tool_done") {
            const toolCallId = (trace.tool_call_id as string | undefined) ?? "";
            const tool = (trace.tool as string | undefined) ?? "";
            const result = trace.result ?? {};
            const isError = (trace.is_error as boolean | undefined) ?? false;
            const durationMs = trace.duration_ms as number | undefined;
            dispatch({
              type: "appendTraceEntry",
              id: blockId,
              entry: {
                kind: "tool_done",
                toolCallId,
                tool,
                result,
                terminal: (trace.terminal as boolean | undefined) ?? false,
                ts: Date.now(),
              },
            });
            // Update the tool_call segment in the content stream
            dispatch({
              type: "updateToolCallSegment",
              id: blockId,
              toolCallId,
              status: isError ? "error" : "done",
              result,
              ...(durationMs != null ? { durationMs } : {}),
            });
            // Record file mutations for the File Changes View
            if (!isError) {
              const fileChange = detectFileChange(tool, trace.args);
              if (fileChange) {
                dispatch({
                  type: "appendFileChange",
                  id: blockId,
                  change: { ...fileChange, blockId, ts: Date.now() },
                });
              }
            }
          } else if (traceKind === "error") {
            // Emitted by the agent loop when the LLM stream itself
            // fails (HTTP error, timeout, etc.). Stash the message so
            // the run_status handler can surface it in place of
            // "(no output)", and add a visible trace entry too.
            const msg = (trace.message as string | undefined) ?? "";
            if (msg) lastErrorMessage = msg;
            dispatch({
              type: "appendTraceEntry",
              id: blockId,
              entry: {
                kind: "tool_start",
                toolCallId: "",
                tool: "error",
                args: {
                  where: (trace.where as string | undefined) ?? "",
                  message: msg || "error",
                },
                ts: Date.now(),
              },
            });
          } else if (traceKind === "review_resolved") {
            const resolvedId = (trace.review_id as string | undefined) ?? "";
            const decision = (trace.decision as string | undefined) === "approve" ? "approve" : "reject";
            dispatch({
              type: "appendTraceEntry",
              id: blockId,
              entry: {
                kind: "approval_resolved",
                reviewId: resolvedId,
                decision,
                ts: Date.now(),
              },
            });
          } else if (traceKind === "reflection") {
            dispatch({
              type: "appendTraceEntry",
              id: blockId,
              entry: {
                kind: "reflection",
                turn: (trace.turn as number | undefined) ?? 0,
                text: (trace.text as string | undefined) ?? "",
                ts: Date.now(),
              },
            });
          } else if (traceKind === "memory_write") {
            dispatch({
              type: "appendTraceEntry",
              id: blockId,
              entry: {
                kind: "memory_write",
                namespace: (trace.namespace as string | undefined) ?? "",
                key: (trace.key as string | undefined) ?? "",
                source: (trace.source as string | undefined) ?? "",
                ts: Date.now(),
              },
            });
          }
        } else if (kind === "log") {
          // Legacy log events — emit as tool_start-like trace entries for visibility
          const message = (pl.message as string | undefined) ?? "";
          if (message) {
            dispatch({
              type: "appendTraceEntry",
              id: blockId,
              entry: {
                kind: "tool_start",
                toolCallId: "",
                tool: "log",
                args: { message },
                ts: Date.now(),
              },
            });
          }
        }
        return;
      }

      if (typeof ev.run_id === "string" && ev.run_id !== runId) return;
      if (ev.kind === "error") {
        const pl2 = (ev.payload as Record<string, unknown> | undefined) ?? {};
        dispatch({
          type: "appendTraceEntry",
          id: blockId,
          entry: {
            kind: "tool_start",
            toolCallId: "",
            tool: "error",
            args: { message: pl2.message ?? "error" },
            ts: Date.now(),
          },
        });
      }
    });

    try {
      // Inject pinned selection comments into the task body
      const commentAtts = block.attachments.filter((a) => a.kind === "comment");
      if (commentAtts.length > 0) {
        const selContext = commentAtts
          .map((a) => {
            const txt = a.selectedText ?? a.label;
            return a.commentText ? `> ${txt}\n[Comment]: ${a.commentText}` : `> ${txt}`;
          })
          .join("\n\n");
        body = `[Referenced selections]\n${selContext}\n\n${body}`;
      }
      // Inject workspace CWD
      if (workspaceRoot) {
        body = `[Workspace: ${workspaceRoot}]\n\n${body}`;
      }
      // Per-message overrides from the chat toolbar; `""` is dropped so
      // the agent/provider default kicks in.
      const runOpts: Parameters<typeof agentRun>[1] = {
        session_id: chatId,
        agent_id: state.selectedFlow ? undefined : state.agentId || undefined,
        flow_id: state.selectedFlow || undefined,
      };
      if (reasoningEffortRef.current) runOpts.reasoning_effort = reasoningEffortRef.current;
      if (anthropicEffortRef.current) runOpts.anthropic_effort = anthropicEffortRef.current;
      // For flow runs, don't forward the UI session-model: flow agents
      // declare their own llm: overrides in YAML; if empty they fall back
      // to the provider's default_model. Sending the UI model here would
      // cause every agent in the flow to use the chat model picker's value,
      // which may be invalid for the active provider.
      if (state.model && !state.selectedFlow) runOpts.model = state.model;
      // If the picked model belongs to a non-active provider group, send
      // that provider's wire config alongside so the request actually
      // routes there instead of being sent to the active provider's
      // endpoint with a model name it doesn't recognise.
      if (state.model && !state.selectedFlow) {
        const owner = modelGroups.find((g) => g.models.includes(state.model));
        if (owner && owner.id !== activeProviderId) {
          runOpts.provider_kind = owner.kind;
          runOpts.base_url = owner.base_url;
          if (owner.api_key) runOpts.api_key = owner.api_key;
        }
      }
      runId = await agentRun(body, runOpts);
      if (!runId) throw new Error("runtime did not return run_id");
      dispatch({ type: "setCurrentRunId", runId });
      await shells.browser.events.subscribe({ run_id: runId }).catch(() => {
        /* ignore */
      });
    } catch (err) {
      off();
      const errMsg = err instanceof Error ? err.message : typeof err === "string" ? err : String(err);
      const isBridgeError = errMsg.includes("bridge invoke failed") || errMsg.includes("send_failed");
      dispatch({
        type: "finalizeBlock",
        id: blockId,
        status: "fail",
      });
      dispatch({
        type: "appendTraceEntry",
        id: blockId,
        entry: {
          kind: "tool_start",
          toolCallId: "",
          tool: "error",
          args: {
            message: isBridgeError
              ? "Runtime is reconnecting. Please wait for the banner to clear, then try again."
              : `Failed to start: ${errMsg}`,
          },
          ts: Date.now(),
        },
      });
      if (isBridgeError) {
        dispatch({ type: "setReconnecting", reconnecting: true });
      }
      dispatch({ type: "clearAwaitingApproval" });
      dispatch({ type: "setRunning", running: false });
      dispatch({ type: "setRunningBlockId", id: null });
      return;
    }

    setTimeout(
      () => {
        off();
        if (state.running) {
          dispatch({ type: "setRunning", running: false });
          dispatch({ type: "setRunningBlockId", id: null });
        }
      },
      5 * 60 * 1000,
    );
  };

  // Finalize shell block when OSC 133 D sets status away from "running"
  useEffect(() => {
    if (!state.runningBlockId) return;
    const blk = state.blocks.find((b) => b.id === state.runningBlockId);
    if (!blk || blk.kind !== "shell") return;
    if (blk.status !== "running") {
      dispatch({ type: "setRunning", running: false });
      dispatch({ type: "setRunningBlockId", id: null });
      if (state.activeChatId) {
        const { data } = loadChatData(state.activeChatId);
        persistChatData(state.activeChatId, { ...data, blocks: state.blocks });
      }
    }
  }, [state.blocks, state.runningBlockId, state.activeChatId, dispatch]);

  // Persist blocks after each conversation finalization (state.running transitions)
  useEffect(() => {
    if (!state.running && state.activeChatId && state.blocks.length > 0) {
      persistChatData(state.activeChatId, {
        blocks: state.blocks,
        terminalTid: state.terminalTid,
        model: state.model,
        agentId: state.agentId || undefined,
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.running]);

  // ── runtime restart recovery ─────────────────────────────────────────
  // When crony restarts: mark the in-flight block as failed, clear terminal
  // session (it will be re-created on demand), and show a reconnecting banner.
  // When all subscriptions are restored, dismiss the banner.
  useEffect(() => {
    const offRestarting = browser.on("runtime.restarting", () => {
      // Fail any in-progress agent block.
      if (state.runningBlockId) {
        dispatch({ type: "finalizeBlock", id: state.runningBlockId, status: "fail" });
        dispatch({ type: "setRunningBlockId", id: null });
      }
      dispatch({ type: "setRunning", running: false });
      dispatch({ type: "clearAwaitingApproval" });
      dispatch({ type: "setReconnecting", reconnecting: true });
    });
    const offReconnected = browser.on("runtime.reconnected", () => {
      dispatch({ type: "setReconnecting", reconnecting: false });
    });
    return () => {
      offRestarting();
      offReconnected();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dispatch, state.runningBlockId]);

  const onShellAction = (action: string, block: ShellBlock) => {
    // Spawn a thread on the block by running a prompt with context
    const prompt = `${action}: \`\`\`\n$ ${block.command}\n${block.output}\n\`\`\``;
    void onRun(prompt);
  };

  const onSubmit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    // If the picker is open, submit should commit the selection, not run the message.
    if (picker) return;
    const typed = inputRef.current?.value.trim() || "";
    if (!typed && attachedPrompts.length === 0) return;
    if (inputRef.current) inputRef.current.value = "";
    setInputMode("chat");
    // Collect content from explicitly-attached picker pills.
    const parts = attachedPrompts.map((p) => p.content);
    const attachedIds = new Set(attachedPrompts.map((p) => p.id));
    // Auto-resolve any /slug tokens typed directly in the textarea.
    // This lets the user type "/opsx-explore <question>" without opening
    // the picker — the content is injected exactly as if picked.
    const pillRe = /^\/([a-z0-9_][a-z0-9_-]*)$/i;
    let remaining = typed;
    for (const word of typed.split(/\s+/)) {
      const m = word.match(pillRe);
      if (!m) continue;
      const hit = workspacePrompts.find((p) => p.label === m[1]);
      if (hit?.content && !attachedIds.has(hit.id)) {
        parts.unshift(hit.content);
        attachedIds.add(hit.id);
        remaining = remaining.replace(word, "").trim();
      }
    }
    if (remaining) parts.push(remaining);
    // Build a concise display label (pill names + typed text, no content dump).
    const displayParts = attachedPrompts.map((p) => `/${p.label}`);
    if (typed) displayParts.push(typed);
    setAttachedPrompts([]);
    void onRun(parts.join("\n\n"), displayParts.join(" ") || typed);
  };

  // ── picker items ────────────────────────────────────────────────────────
  const pickerItems = useMemo<PickerItem[]>(() => {
    if (!picker) return [];
    const q = picker.query.toLowerCase();
    if (picker.type === "slash") {
      const custom = loadCustomPrompts();
      const all = [...BUILTIN_COMMANDS, ...custom, ...workspacePrompts];
      return all.filter((x) => !q || x.label.toLowerCase().startsWith(q)).slice(0, 8);
    } else {
      // "at" type: filter agents
      return state.agents
        .filter((a) => !q || a.name.toLowerCase().includes(q))
        .map((a) => ({
          id: a.name,
          label: a.name,
          description: a.kind ? `kind: ${a.kind}` : undefined,
        }))
        .slice(0, 8);
    }
  }, [picker, state.agents, workspacePrompts]);

  /** Commit a selected picker item, updating the textarea value. */
  const commitPickerItem = (item: PickerItem) => {
    const el = inputRef.current;
    if (!el || !picker) return;

    if (picker.type === "slash") {
      // Replace the "/query" token with nothing (the action handles the rest)
      el.value = el.value.slice(0, picker.triggerStart) + el.value.slice(el.selectionStart ?? el.value.length);
      el.style.height = "auto";
      el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
      setPicker(null);
      setInputMode("chat");
      el.focus();
      if (item.action === "clear") {
        // Execute the clear command inline
        dispatch({ type: "clearHistory" });
        if (state.activeChatId) {
          persistChatData(state.activeChatId, {
            blocks: [],
            terminalTid: state.terminalTid,
            model: state.model,
          });
        }
      } else if (item.action === "new") {
        // Focus textarea so user can start a fresh message (new chat
        // creation is handled by the tab manager on the host side).
        el.value = "";
        el.style.height = "auto";
      } else if (item.content) {
        // Always attach as a pill so the user can type additional context
        // before submitting. The pill content is prepended to their message
        // in onSubmit. Focus the textarea so they can start typing immediately.
        setAttachedPrompts((prev) => {
          if (prev.some((p) => p.id === item.id)) return prev;
          return [...prev, { id: item.id, label: item.label, content: item.content! }];
        });
      }
    } else {
      // "@" picker: splice in "@AgentName "
      const prefix = el.value.slice(0, picker.triggerStart);
      const suffix = el.value.slice(el.selectionStart ?? el.value.length);
      const replacement = `@${item.label} `;
      el.value = prefix + replacement + suffix;
      // Move cursor after the inserted mention
      const newCaret = picker.triggerStart + replacement.length;
      el.setSelectionRange(newCaret, newCaret);
      el.style.height = "auto";
      el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
      setPicker(null);
      el.focus();
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // When picker is open, intercept navigation keys
    if (picker && pickerItems.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setPickerIdx((i) => Math.min(i + 1, pickerItems.length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setPickerIdx((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        const item = pickerItems[pickerIdx];
        if (item) commitPickerItem(item);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setPicker(null);
        setInputMode("chat");
        return;
      }
      if (e.key === "Tab") {
        e.preventDefault();
        const item = pickerItems[pickerIdx];
        if (item) commitPickerItem(item);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      e.currentTarget.form?.requestSubmit();
    }
  };

  const onClear = () => {
    dispatch({ type: "clearHistory" });
    if (state.activeChatId) {
      persistChatData(state.activeChatId, {
        blocks: [],
        terminalTid: state.terminalTid,
        model: state.model,
      });
    }
  };

  const onRestoreBlock = (blockId: string) => {
    const warned = sessionStorage.getItem("cronymax.restore_warned");
    if (!warned) {
      sessionStorage.setItem("cronymax.restore_warned", "1");
      const ok = window.confirm(
        "Restore to this checkpoint? All conversation blocks after this point will be permanently removed.",
      );
      if (!ok) return;
    }
    dispatch({ type: "restoreToBlock", blockId });
    if (state.activeChatId) {
      const restored = state.blocks.slice(0, state.blocks.findIndex((b) => b.id === blockId) + 1);
      persistChatData(state.activeChatId, {
        blocks: restored,
        terminalTid: state.terminalTid,
        model: state.model,
      });
    }
  };

  const onForkBlock = (blockId: string) => {
    const idx = state.blocks.findIndex((b) => b.id === blockId);
    if (idx < 0) return;
    const slice = state.blocks.slice(0, idx + 1);

    // Create a new chat entry in the chats list
    const newId = `c${Date.now().toString(36)}f`;
    const newName = `${state.chatName} (fork)`;
    try {
      const existing = loadChatsList();
      localStorage.setItem("chats", JSON.stringify([...existing, { id: newId, name: newName }]));
      sessionStorage.setItem("cronymax_chat_tab_id", newId);
    } catch {
      /* ignore */
    }

    persistChatData(newId, { blocks: slice, terminalTid: null, model: state.model });
    dispatch({
      type: "loadChat",
      id: newId,
      name: newName,
      blocks: slice,
      terminalTid: null,
      model: state.model,
      agentId: state.agentId,
    });
  };

  const runningBlockId = state.runningBlockId;

  // Copilot-like textarea auto-height
  const onTextareaInput = (e: React.FormEvent<HTMLTextAreaElement>) => {
    const el = e.currentTarget;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  };

  return (
    <TooltipProvider>
      <main className="flex h-screen flex-col bg-background text-foreground">
        {/* Header */}
        <header className="flex items-center gap-3 border-b border-border bg-card px-3 py-2">
          <Heading className="flex-1 truncate">{state.chatName}</Heading>

          <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
            Flow:
            <Select
              value={state.selectedFlow}
              onValueChange={(v) => {
                dispatch({ type: "setSelectedFlow", name: v });
                persistSelectedFlow(v);
              }}
            >
              <SelectTrigger size="sm" className="max-w-[140px]">
                <SelectValue placeholder="(no flows)" />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {state.flows.map((n) => (
                    <SelectItem key={n} value={n}>
                      {n}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>

          <Button type="button" variant="outline" size="sm" onClick={onClear}>
            Clear
          </Button>
        </header>

        {/* Migration notice */}
        {state.migrationNotice && (
          <Alert className="rounded-none border-0 border-b">
            <Info />
            <AlertDescription className="flex items-center gap-2">
              <span className="flex-1">{state.migrationNotice}</span>
              <Button
                variant="ghost"
                size="icon-xs"
                onClick={() => dispatch({ type: "clearMigrationNotice" })}
                aria-label="Dismiss"
              >
                <X />
              </Button>
            </AlertDescription>
          </Alert>
        )}

        {/* Runtime reconnecting banner */}
        {state.isReconnecting && (
          <Alert className="rounded-none border-0 border-b">
            <Loader2 className="animate-spin" />
            <AlertDescription className="flex items-center gap-2">
              <span className="flex-1">Reconnecting to runtime…</span>
              <Tip tip="Check connection">
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label="Check connection"
                  onClick={() => {
                    void shells.browser.space
                      .list()
                      .then(() => dispatch({ type: "setReconnecting", reconnecting: false }))
                      .catch(() => {
                        /* still reconnecting — keep banner */
                      });
                  }}
                >
                  <X />
                </Button>
              </Tip>
            </AlertDescription>
          </Alert>
        )}

        {/* Agent load error */}
        {agentLoadError && (
          <Alert variant="destructive" className="rounded-none border-0 border-b">
            <TriangleAlert />
            <AlertTitle>agent.registry.list failed</AlertTitle>
            <AlertDescription>{agentLoadError}</AlertDescription>
          </Alert>
        )}

        {/* Block timeline */}
        <div ref={timelineRef} className="flex flex-1 flex-col gap-3 overflow-y-auto px-4 py-2">
          {state.blocks.map((b) => (
            <BlockView
              key={b.id}
              block={b}
              isStreaming={b.id === runningBlockId && b.kind === "conversation"}
              onShellAction={onShellAction}
              isHighlighted={b.id === highlightedBlockId}
              workspacePrompts={workspacePrompts}
              onRestoreBlock={!state.running ? onRestoreBlock : undefined}
              onForkBlock={!state.running ? onForkBlock : undefined}
            />
          ))}
        </div>

        {/* ── Floating selection tooltip ──────────────────────────────
          Anchored to the selection rect via PopoverAnchor (zero-pointer-
          events div positioned at the rect). Radix's popper handles edge
          collision, flip, and Portal rendering — so the tooltip never gets
          clipped by the webview's borders or by any ancestor's overflow. */}
        {activeSelection && (
          <Popover open modal={false}>
            <PopoverAnchor asChild>
              <div
                aria-hidden
                className="pointer-events-none fixed"
                style={{
                  top: activeSelection.anchorRect.top,
                  left: activeSelection.anchorRect.left,
                  width: activeSelection.anchorRect.width,
                  height: activeSelection.anchorRect.height,
                }}
              />
            </PopoverAnchor>
            <PopoverContent
              side="top"
              sideOffset={4}
              align="center"
              collisionPadding={8}
              // Keep the user's text selection alive: don't auto-focus when the
              // popover opens, and don't return focus on close.
              onOpenAutoFocus={(e) => e.preventDefault()}
              onCloseAutoFocus={(e) => e.preventDefault()}
              onMouseDown={(e) => {
                // Always prevent default to keep text selection alive.
                // Manually focus inputs so they still receive keyboard events.
                e.preventDefault();
                if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) {
                  e.target.focus();
                }
              }}
              className="flex w-auto flex-col gap-1 p-1.5"
            >
              {/* Quick actions row */}
              <div className="flex items-center gap-0.5">
                <Button
                  variant="ghost"
                  size="xs"
                  onClick={() => navigator.clipboard.writeText(activeSelection.selectedText)}
                >
                  <Copy data-icon="inline-start" />
                  Copy
                </Button>
                <Button
                  size="xs"
                  onClick={() => {
                    const commentId = crypto.randomUUID();
                    dispatch({
                      type: "pinComment",
                      comment: {
                        id: commentId,
                        blockId: activeSelection.blockId,
                        selectedText: activeSelection.selectedText,
                        text: commentDraft.trim() || undefined,
                        pinnedToPrompt: true,
                      },
                    });
                    setCommentDraft("");
                    setFrozenSelection(null);
                    window.getSelection()?.removeAllRanges();
                  }}
                >
                  <Pin data-icon="inline-start" />
                  Pin
                </Button>
              </div>
              {/* Comment input — base Textarea applies `field-sizing-content`
                which sizes the box to its content, so the `rows` attr is
                ignored once the user types. Pin a real CSS floor instead:
                2 lines of text-xs (line-height 1rem) + py-2 padding +
                2 × 1px border ≈ 3.25rem. `max-h-32` caps runaway growth
                with internal scroll. */}
              <Textarea
                rows={2}
                value={commentDraft}
                onChange={(e) => setCommentDraft(e.target.value)}
                placeholder="Add a comment… (Enter to pin, Shift+Enter for newline)"
                className="max-h-32 min-h-[3.25rem] w-64 resize-none text-xs"
                onFocus={() => setFrozenSelection(selectionInfo ?? frozenSelection)}
                onBlur={() => setFrozenSelection(null)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    const commentId = crypto.randomUUID();
                    dispatch({
                      type: "pinComment",
                      comment: {
                        id: commentId,
                        blockId: activeSelection.blockId,
                        selectedText: activeSelection.selectedText,
                        text: commentDraft.trim() || undefined,
                        pinnedToPrompt: true,
                      },
                    });
                    setCommentDraft("");
                    setFrozenSelection(null);
                    window.getSelection()?.removeAllRanges();
                  }
                  if (e.key === "Escape") {
                    setCommentDraft("");
                    setFrozenSelection(null);
                    window.getSelection()?.removeAllRanges();
                  }
                }}
              />
            </PopoverContent>
          </Popover>
        )}

        {/* ── Flow instances bar — visible when session has active flow runs ── */}
        <FlowInstancesBar sessionId={state.activeChatId} />

        {/* ── File changes summary ─────────────────────────────────────────── */}
        <FileChangesView changes={sessionFileChanges} />

        {/* ── Copilot-like composer ──────────────────────────────────── */}
        <form onSubmit={onSubmit} className="px-3 pb-1 pt-1">
          {/* Approval card — shown when agent awaits tool review */}
          {state.awaitingApproval && (
            <ApprovalCard
              runId={state.awaitingApproval.runId}
              reviewId={state.awaitingApproval.reviewId}
              toolName={state.awaitingApproval.toolName}
              args={state.awaitingApproval.args}
              onAllow={() => dispatch({ type: "clearAwaitingApproval" })}
              onDeny={() => dispatch({ type: "clearAwaitingApproval" })}
            />
          )}

          {/* Attachment tray sits above the editor box */}
          <AttachmentTray
            attachments={state.attachments}
            onRemove={(id) => dispatch({ type: "removeAttachment", id })}
            onCommentClick={onCommentAttachmentClick}
          />

          {/* Picker + editor wrapper — relative so the picker floats above */}
          <div className="relative">
            {/* ── Reviews panel — floats above editor when pending approvals exist ── */}
            <div className="absolute bottom-full left-0 right-0 z-40 mb-1">
              <ReviewsPanel sessionId={state.activeChatId} />
            </div>

            {/* ── Slash / @ picker ──────────────────────────────────────── */}
            {picker && pickerItems.length > 0 && (
              <div className="absolute inset-x-0 bottom-full z-50 mb-1 overflow-hidden rounded-lg border border-border bg-popover text-popover-foreground shadow-lg">
                <div className="px-2 pb-0.5 pt-1.5 text-xs font-medium text-muted-foreground">
                  {picker.type === "slash" ? "Commands" : "Agents"}
                </div>
                {pickerItems.map((item, idx) => (
                  <button
                    key={item.id}
                    type="button"
                    data-active={idx === pickerIdx}
                    className={cn(
                      "flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs transition",
                      idx === pickerIdx
                        ? "bg-accent text-accent-foreground"
                        : "text-muted-foreground hover:bg-accent/50 hover:text-foreground",
                    )}
                    onMouseEnter={() => setPickerIdx(idx)}
                    onMouseDown={(e) => {
                      // Use onMouseDown + preventDefault so the textarea doesn't blur
                      e.preventDefault();
                      commitPickerItem(item);
                    }}
                  >
                    <span className="w-5 shrink-0 text-center font-mono font-semibold text-primary">
                      {picker.type === "slash" ? "/" : "@"}
                    </span>
                    <span className="font-semibold">{item.label}</span>
                    {item.description && (
                      <span className="ml-1 truncate text-muted-foreground">— {item.description}</span>
                    )}
                  </button>
                ))}
              </div>
            )}

            {/* Editor card */}
            <div
              className={cn(
                "flex flex-col rounded-xl border bg-card transition-colors",
                inputMode === "shell" ? "border-amber-500/70 bg-amber-500/5" : "border-border focus-within:border-ring",
              )}
            >
              {/* Attached prompt pills (VS-Code-style slash command references) */}
              {attachedPrompts.length > 0 && (
                <div className="relative flex flex-wrap gap-1 px-2.5 pb-0 pt-2">
                  {/* PromptPopover rendered above the pill row */}
                  {activePillId !== null &&
                    (() => {
                      const activePill = attachedPrompts.find((p) => p.id === activePillId);
                      if (!activePill) return null;
                      return (
                        <PromptPopover
                          key={activePillId}
                          prompt={activePill}
                          onClose={() => setActivePillId(null)}
                          onSave={async (label, content) => {
                            const res = await shells.browser.workspace.prompt.save({ name: label, content });
                            if (!res.ok) throw new Error(res.error ?? "Save failed");
                            setAttachedPrompts((prev) =>
                              prev.map((p) => (p.id === activePillId ? { ...p, content } : p)),
                            );
                            setActivePillId(null);
                          }}
                        />
                      );
                    })()}
                  {attachedPrompts.map((p) => (
                    <button
                      type="button"
                      key={p.id}
                      className={cn(
                        "inline-flex cursor-pointer items-center gap-1 rounded-md border px-1.5 py-0.5 font-mono text-xs text-primary transition",
                        activePillId === p.id
                          ? "border-primary/60 bg-primary/25"
                          : "border-primary/30 bg-primary/15 hover:bg-primary/25",
                      )}
                      onClick={() => setActivePillId((prev) => (prev === p.id ? null : p.id))}
                    >
                      <span className="opacity-70">/</span>
                      {p.label}
                      <button
                        type="button"
                        className="ml-0.5 leading-none opacity-50 hover:opacity-100"
                        onClick={(e) => {
                          e.stopPropagation();
                          setActivePillId((prev) => (prev === p.id ? null : prev));
                          setAttachedPrompts((prev) => prev.filter((x) => x.id !== p.id));
                        }}
                        aria-label="Detach prompt"
                      >
                        <X className="size-3" />
                      </button>
                    </button>
                  ))}
                </div>
              )}

              {/* Prefix badge row (shown when mode ≠ chat) */}
              {inputMode !== "chat" && (
                <div className="flex items-center gap-1.5 px-3 pb-0 pt-2">
                  <Badge
                    variant="secondary"
                    className={cn(
                      "font-mono",
                      inputMode === "shell" && "bg-amber-500/20 text-amber-600 dark:text-amber-300",
                    )}
                  >
                    {inputMode === "shell" ? "$ shell" : "/ command"}
                  </Badge>
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    className="ml-auto"
                    onClick={() => {
                      if (inputRef.current) inputRef.current.value = "";
                      setInputMode("chat");
                      setPicker(null);
                      setAttachedPrompts([]);
                    }}
                    aria-label="Reset mode"
                  >
                    <X />
                  </Button>
                </div>
              )}

              {/* Textarea */}
              <Textarea
                ref={inputRef}
                rows={1}
                disabled={!!state.awaitingApproval}
                placeholder={
                  state.awaitingApproval
                    ? "Waiting for tool approval…"
                    : inputMode === "shell"
                      ? "shell command…"
                      : inputMode === "command"
                        ? "command…"
                        : "Ask anything… (@AgentName to address one, $ for shell, / for commands)"
                }
                onKeyDown={onKeyDown}
                onChange={onInputChange}
                onInput={onTextareaInput}
                onPaste={onPaste}
                className="min-h-0 resize-none rounded-none border-0 bg-transparent px-3 py-2.5 text-sm shadow-none focus-visible:border-0 focus-visible:ring-0 dark:bg-transparent"
              />

              {/* Bottom toolbar row */}
              <div className="flex items-center gap-1.5 px-2 pb-2">
                {/* Add button */}
                <Tip tip="Add file / image">
                  <Button type="button" variant="outline" size="sm" onClick={() => fileInputRef.current?.click()}>
                    <Plus data-icon="inline-start" />
                    Add
                  </Button>
                </Tip>
                <input ref={fileInputRef} type="file" className="hidden" multiple onChange={onFileChange} />

                {/* Model combobox */}
                <Popover open={modelComboOpen} onOpenChange={setModelComboOpen}>
                  <Tip tip="LLM model">
                    <PopoverTrigger asChild>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="max-w-[140px] justify-between font-normal text-muted-foreground"
                      >
                        <span className="truncate">{state.model || "provider default"}</span>
                        <ChevronsUpDown data-icon="inline-end" className="opacity-50" />
                      </Button>
                    </PopoverTrigger>
                  </Tip>
                  <PopoverContent className="w-[220px] p-0" align="start" side="top">
                    <Command>
                      <CommandInput placeholder="Search models…" className="h-7 text-xs" />
                      <CommandList>
                        <CommandEmpty className="text-xs">No models.</CommandEmpty>
                        <CommandGroup>
                          <CommandItem
                            value=""
                            onSelect={() => {
                              dispatch({ type: "setModel", model: "" });
                              persistSelectedModel("");
                              setModelComboOpen(false);
                            }}
                            className="text-xs"
                          >
                            <Check className={cn("mr-2 size-3 shrink-0", state.model ? "opacity-0" : "opacity-100")} />
                            <span className="italic text-muted-foreground">provider default</span>
                          </CommandItem>
                        </CommandGroup>
                        {modelGroups.map((g) => (
                          <CommandGroup key={g.label} heading={g.label}>
                            {g.models.map((m) => (
                              <CommandItem
                                key={m}
                                value={m}
                                onSelect={(v) => {
                                  dispatch({ type: "setModel", model: v });
                                  persistSelectedModel(v);
                                  setModelComboOpen(false);
                                }}
                                className="text-xs"
                              >
                                <Check
                                  className={cn(
                                    "mr-2 size-3 shrink-0",
                                    m === state.model ? "opacity-100" : "opacity-0",
                                  )}
                                />
                                <span className="truncate font-mono">{m}</span>
                              </CommandItem>
                            ))}
                          </CommandGroup>
                        ))}
                      </CommandList>
                    </Command>
                  </PopoverContent>
                </Popover>

                {/* Effort selector — provider-kind aware. Anthropic uses its
                  own enum (low/medium/high/max), OpenAI uses
                  minimal/low/medium/high/xhigh. Copilot proxies the underlying
                  model and ignores both, so we hide the selector. Unknown
                  providers default to the OpenAI selector. */}
                {(() => {
                  // Prefer the kind of the group containing the selected model;
                  // fall back to the currently-active provider's kind (covers
                  // empty `state.model` = "provider default" and unrecognised
                  // model strings).
                  const currentKind =
                    modelGroups.find((g) => g.models.includes(state.model))?.kind || activeProviderKind;
                  if (currentKind === "github_copilot") return null;
                  // Radix Select disallows empty-string item values, so we
                  // round-trip `""` (provider default) through a sentinel.
                  const EFFORT_DEFAULT = "__default__";
                  if (currentKind === "anthropic") {
                    return (
                      <Select
                        value={anthropicEffort || EFFORT_DEFAULT}
                        onValueChange={(v) => {
                          const next = (v === EFFORT_DEFAULT ? "" : v) as AnthropicEffort;
                          setAnthropicEffortState(next);
                          persistAnthropicEffort(next);
                        }}
                      >
                        <Tip tip="Anthropic adaptive thinking effort (claude-* models)">
                          <SelectTrigger size="sm" className="text-xs text-muted-foreground">
                            <SelectValue />
                          </SelectTrigger>
                        </Tip>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value={EFFORT_DEFAULT}>think: default</SelectItem>
                            <SelectItem value="low">think: low</SelectItem>
                            <SelectItem value="medium">think: medium</SelectItem>
                            <SelectItem value="high">think: high</SelectItem>
                            <SelectItem value="max">think: max</SelectItem>
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    );
                  }
                  return (
                    <Select
                      value={reasoningEffort || EFFORT_DEFAULT}
                      onValueChange={(v) => {
                        const next = (v === EFFORT_DEFAULT ? "" : v) as ReasoningEffort;
                        setReasoningEffortState(next);
                        persistReasoningEffort(next);
                      }}
                    >
                      <Tip tip="Reasoning effort (OpenAI gpt-5 / o-series)">
                        <SelectTrigger size="sm" className="text-xs text-muted-foreground">
                          <SelectValue />
                        </SelectTrigger>
                      </Tip>
                      <SelectContent>
                        <SelectGroup>
                          <SelectItem value={EFFORT_DEFAULT}>think: default</SelectItem>
                          <SelectItem value="minimal">think: minimal</SelectItem>
                          <SelectItem value="low">think: low</SelectItem>
                          <SelectItem value="medium">think: medium</SelectItem>
                          <SelectItem value="high">think: high</SelectItem>
                          <SelectItem value="xhigh">think: xhigh</SelectItem>
                        </SelectGroup>
                      </SelectContent>
                    </Select>
                  );
                })()}

                <div className="flex-1" />

                {/* Send / Stop button */}
                {state.running ? (
                  <Tip tip="Stop run">
                    <Button
                      type="button"
                      size="icon-sm"
                      variant="destructive"
                      aria-label="Stop"
                      onClick={() => {
                        if (state.currentRunId) {
                          flowRun.cancel(state.currentRunId).catch(() => undefined);
                        }
                      }}
                    >
                      <Square />
                    </Button>
                  </Tip>
                ) : (
                  <Tip tip="Send (Enter)">
                    <Button type="submit" size="icon-sm" disabled={state.isReconnecting} aria-label="Send">
                      <ArrowUp />
                    </Button>
                  </Tip>
                )}
              </div>
            </div>
          </div>
          {/* end relative picker wrapper */}
        </form>

        {/* ── Below-editor status bar: approval mode + context hint ──────────── */}
        <div className="flex items-center gap-2 px-3 pb-3 pt-0">
          {/* Global approval mode — semantic shadcn Select matches the effort
            selectors above. Triggers in line with the composer toolbar via
            `size="sm"`. */}
          <Select
            value={globalApprovalMode}
            onValueChange={(v) => {
              const next = v as "default" | "autopilot" | "bypass";
              setGlobalApprovalMode(next);
              persistGlobalApprovalMode(next);
            }}
          >
            <Tip tip="Global approval mode — overrides per-tool trust for this session">
              <SelectTrigger size="sm" className="border-0 bg-transparent text-xs text-muted-foreground">
                <SelectValue />
              </SelectTrigger>
            </Tip>
            <SelectContent>
              <SelectGroup>
                <SelectItem value="default">approve: per-tool</SelectItem>
                <SelectItem value="autopilot">approve: all</SelectItem>
                <SelectItem value="bypass">approve: none</SelectItem>
              </SelectGroup>
            </SelectContent>
          </Select>

          {/* Context window hint — `warn` (≥80 %) keeps amber as the only
            intentional non-semantic accent (matches shell-mode composer
            border); `critical` (≥95 %) uses `text-destructive` /
            `bg-destructive/10` semantic tokens so it auto-flips with theme. */}
          {latestUsage &&
            contextLimit &&
            (() => {
              const used = latestUsage.inputTokens + latestUsage.outputTokens;
              const pct = Math.min(100, Math.round((used / contextLimit) * 100));
              const warn = pct >= 80;
              const critical = pct >= 95;
              return (
                <div
                  className={cn(
                    "flex items-center gap-2 rounded-md px-2 py-1 text-[11px]",
                    critical
                      ? "bg-destructive/10 text-destructive"
                      : warn
                        ? "bg-amber-500/10 text-amber-600 dark:text-amber-300"
                        : "bg-muted/40 text-muted-foreground",
                  )}
                  title={`Input: ${latestUsage.inputTokens.toLocaleString()} tokens · Output: ${latestUsage.outputTokens.toLocaleString()} tokens`}
                >
                  <div className="h-1 w-16 overflow-hidden rounded-full bg-current/20">
                    <div
                      className={cn(
                        "h-full rounded-full transition-all",
                        critical ? "bg-destructive" : warn ? "bg-amber-500" : "bg-primary/50",
                      )}
                      style={{ width: `${pct}%` }}
                    />
                  </div>
                  <span className="shrink-0 tabular-nums">
                    {pct}% of {(contextLimit / 1_000).toFixed(0)}k ctx
                  </span>
                </div>
              );
            })()}
        </div>
      </main>
    </TooltipProvider>
  );
}
