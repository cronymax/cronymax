import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@/styles/theme.css";
import { installThemeMirror } from "@/styles/installThemeMirror";

installThemeMirror();

import { ErrorBoundary } from "@/components/ErrorBoundary";
import { startThemeSampler } from "@/theme_sampler";
import { App } from "./App";

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("root element missing");

startThemeSampler();

createRoot(rootEl).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
