import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@/styles/theme.css";
import { installPanelMode } from "@/styles/installPanelMode";
import { installThemeMirror } from "@/styles/installThemeMirror";

installThemeMirror();
installPanelMode();

import { ErrorBoundary } from "@/components/ErrorBoundary";
import { App } from "./App";

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("root element missing");

createRoot(rootEl).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
