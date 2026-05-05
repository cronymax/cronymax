import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@/styles/theme.css";
import "streamdown/styles.css";
import { installThemeMirror } from "@/styles/installThemeMirror";

installThemeMirror();
import { startThemeSampler } from "@/theme_sampler";
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
