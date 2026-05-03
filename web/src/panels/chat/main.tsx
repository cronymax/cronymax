import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@/styles/theme.css";
import { installThemeMirror } from "@/styles/installThemeMirror";

installThemeMirror();
import { startThemeSampler } from "@/theme_sampler";
// Legacy ReAct runtime — side-effect imports register window.llmClient,
// window.toolBridge, window.AgentGraph (load order matters: llm → tools → loop).
import "@/agent_runtime/llm.js";
import "@/agent_runtime/tools.js";
import "@/agent_runtime/loop.js";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { App } from "./App";
import { Provider } from "./store";

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("root element missing");

startThemeSampler();

createRoot(rootEl).render(
  <StrictMode>
    <ErrorBoundary>
      <Provider>
        <App />
      </Provider>
    </ErrorBoundary>
  </StrictMode>,
);
