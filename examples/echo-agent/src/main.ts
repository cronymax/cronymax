// Echo agent — minimal `cronymax.agents.provider` example.
//
// **Runtime note:** bootstrap.js injects the live cronymax namespace on
// `globalThis.cronymax` and does NOT install a `require()` hook for
// `@cronymax/extension`. So we import the SDK *types* only (so the editor
// and `tsc` check the IDL contract) and access the live values via the
// global. The SDK's `runtime` re-export will start working once bootstrap
// learns to shim `require("@cronymax/extension")`; until then, the global
// is the only path that's wired end-to-end.

import type {
  AgentEvent,
  AgentProvider,
  AgentSession,
  CancellationToken,
  ContributionItem,
  Cronymax,
  ExtensionContext,
  LogOutputChannel,
  PermissionDecision,
  PromptMessage,
  SessionOptions,
} from "@cronymax/extension";

// biome-ignore lint/suspicious/noShadowRestrictedNames: bootstrap injects `cronymax` onto globalThis; we shadow the type here so the SDK API is visible without a require() shim.
declare const globalThis: { cronymax: Cronymax };

const PROVIDER_ID = "cronymax-examples.echo-agent.echo";

// Module-level log channel so the session can log too (set in `activate`).
// Doubles as a demo of the settings → Extensions → Logs tab: a turn emits a
// few lines at different levels (info / warn / debug) via this channel.
let log: LogOutputChannel | null = null;

// Model id that exercises the permission round-trip — the session yields
// a `permissionRequest` event, parks on `resolvePermission`, and only
// echoes the reply once the host returns a decision.
const PERMISSION_MODEL = "echo-permission";

class EchoSession implements AgentSession {
  readonly id: string;
  readonly model: string;
  private cancelled = false;
  // Pending permission deciders keyed by requestId. Populated by
  // `prompt()` right before it yields a `permissionRequest`; drained by
  // `resolvePermission()` when the host wires the user's decision back.
  private readonly pendingDecisions = new Map<string, (decision: PermissionDecision) => void>();

  constructor(id: string, model: string) {
    this.id = id;
    this.model = model;
  }

  async *prompt(message: PromptMessage, token: CancellationToken): AsyncIterable<AgentEvent> {
    const preview = message.text.length > 60 ? `${message.text.slice(0, 60)}…` : message.text;
    log?.info(`prompt: model=${this.model || "(default)"} text=${JSON.stringify(preview)}`);

    // When the picker selected the permission model, gate the echo
    // behind a host-side permission decision. The `await` below parks
    // the iterator until `resolvePermission()` resolves the matching
    // pending entry — that's the round-trip the chat panel's
    // ApprovalCard drives end-to-end.
    if (this.model === PERMISSION_MODEL) {
      const requestId = `echo-perm-${Date.now()}-${Math.floor(Math.random() * 1e6)}`;
      const decisionPromise = new Promise<PermissionDecision>((resolve) => {
        this.pendingDecisions.set(requestId, resolve);
      });
      log?.warn(`permission requested: echo.reply (${requestId})`);
      yield {
        kind: "permissionRequest",
        requestId,
        tool: "echo.reply",
        options: { preview: message.text },
      };
      const decision = await decisionPromise;
      log?.info(`permission ${decision.allow ? "granted" : "denied"}`);
      if (this.cancelled || token.isCancellationRequested) {
        log?.debug("cancelled while awaiting permission");
        yield { kind: "done", stopReason: "cancelled" };
        return;
      }
      if (!decision.allow) {
        yield { kind: "text", text: "(echo declined: permission denied)" };
        yield { kind: "done", stopReason: "end_turn" };
        return;
      }
    }

    const header = `[model=${this.model || "(default)"}] `;
    const reply = `${header}Echo: ${message.text}`;

    log?.debug(`streaming ${reply.length} chars in ${Math.ceil(reply.length / 8)} chunks`);
    for (const chunk of chunkText(reply, 8)) {
      if (this.cancelled || token.isCancellationRequested) {
        log?.debug("cancelled mid-stream");
        yield { kind: "done", stopReason: "cancelled" };
        return;
      }
      yield { kind: "text", text: chunk };
      await delay(40);
    }

    log?.info("turn complete");
    yield { kind: "done", stopReason: "end_turn" };
  }

  async resolvePermission(requestId: string, decision: PermissionDecision): Promise<void> {
    const resolver = this.pendingDecisions.get(requestId);
    if (!resolver) return;
    this.pendingDecisions.delete(requestId);
    resolver(decision);
  }

  async cancel(): Promise<void> {
    this.cancelled = true;
    // Unblock any parked permission awaits so the iterator can drain.
    for (const [requestId, resolver] of this.pendingDecisions) {
      resolver({ allow: false });
      this.pendingDecisions.delete(requestId);
    }
  }

  async dispose(): Promise<void> {
    await this.cancel();
  }
}

const echoProvider: AgentProvider = {
  async enumerate(): Promise<readonly ContributionItem[]> {
    return [
      { id: "echo-fast", label: "Echo (fast)", description: "Tight 8-char chunks" },
      { id: "echo-verbose", label: "Echo (verbose)", description: "Same echo, longer header" },
      { id: "echo-default", label: "Echo (default)" },
      {
        id: PERMISSION_MODEL,
        label: "Echo (with permission)",
        description: "Requests host permission before echoing.",
      },
    ];
  },
  async createSession(opts: SessionOptions): Promise<AgentSession> {
    return new EchoSession(`echo-${Date.now()}-${Math.floor(Math.random() * 1e6)}`, opts.model ?? "");
  },
};

export async function activate(ctx: ExtensionContext): Promise<void> {
  const c = globalThis.cronymax;
  log = c.window.createOutputChannel("Echo Agent", { log: true });
  ctx.subscriptions.push(log);
  log.info("echo-agent activate()");

  ctx.subscriptions.push(c.agents.registerProvider(PROVIDER_ID, echoProvider));
}

export async function deactivate(): Promise<void> {
  // ctx.subscriptions are disposed by the host.
  log = null;
}

function chunkText(text: string, size: number): string[] {
  if (size <= 0) return [text];
  const out: string[] = [];
  for (let i = 0; i < text.length; i += size) {
    out.push(text.slice(i, i + size));
  }
  return out;
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
