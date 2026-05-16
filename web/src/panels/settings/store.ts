/**
 * Agent panel store. The actual ReAct loop runs in the legacy global
 * runtime (`window.AgentGraph` / `window.llmClient`); this store tracks
 * UI-side concerns: status, accumulated trace text, settings overlay,
 * permission overlay, and the spaces sidebar.
 */
import { createPanelStore } from "@/hooks/usePanelStore";
import type { Space } from "@/types";

export type Status = "idle" | "running" | "done" | "failed";
export type ConfigTab = "flows" | "agents" | "workspace" | "providers" | "runner";

export interface PermissionRequest {
  prompt: string;
  /** Internal request id (graph-local). */
  requestId: string | null;
  /** Resolver supplied by the legacy graph. */
  resolve: ((allow: boolean) => void) | null;
}

export interface State {
  status: Status;
  task: string;
  result: string;
  spaces: Space[];
  activeSpaceId: string | null;
  llmBaseUrl: string;
  llmApiKey: string;
  llmModel: string;
  permission: PermissionRequest | null;
  /** Active Config-page tab. */
  tab: ConfigTab;
}

export type Action =
  | { type: "setTab"; tab: ConfigTab }
  | { type: "setStatus"; status: Status }
  | { type: "setTask"; task: string }
  | { type: "appendResult"; chunk: string }
  | { type: "resetResult" }
  | { type: "setSpaces"; spaces: Space[] }
  | { type: "setActiveSpace"; id: string | null }
  | { type: "setLlmConfig"; baseUrl: string; apiKey: string; model: string }
  | {
      type: "updateLlmField";
      field: "baseUrl" | "apiKey" | "model";
      value: string;
    }
  | { type: "requestPermission"; req: PermissionRequest }
  | { type: "clearPermission" };

const initial: State = {
  status: "idle",
  task: "",
  result: "",
  spaces: [],
  activeSpaceId: null,
  llmBaseUrl: "",
  llmApiKey: "",
  llmModel: "gpt-4o-mini",
  permission: null,
  tab: "flows",
};

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "setTab":
      return { ...state, tab: action.tab };
    case "setStatus":
      return { ...state, status: action.status };
    case "setTask":
      return { ...state, task: action.task };
    case "appendResult":
      return { ...state, result: state.result + action.chunk };
    case "resetResult":
      return { ...state, result: "" };
    case "setSpaces":
      return {
        ...state,
        spaces: action.spaces,
        activeSpaceId: state.activeSpaceId ?? (action.spaces.length > 0 ? action.spaces[0]!.id : null),
      };
    case "setActiveSpace":
      return { ...state, activeSpaceId: action.id };
    case "setLlmConfig":
      return {
        ...state,
        llmBaseUrl: action.baseUrl,
        llmApiKey: action.apiKey,
        llmModel: action.model,
      };
    case "updateLlmField":
      if (action.field === "baseUrl") return { ...state, llmBaseUrl: action.value };
      if (action.field === "apiKey") return { ...state, llmApiKey: action.value };
      return { ...state, llmModel: action.value };
    case "requestPermission":
      return { ...state, permission: action.req };
    case "clearPermission":
      return { ...state, permission: null };
    default:
      return state;
  }
}

export const { Provider, useStore } = createPanelStore<State, Action>(reducer, initial);
