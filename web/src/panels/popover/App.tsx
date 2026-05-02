import { useState, useEffect } from "react";
import { bridge } from "@/bridge";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";

const initialUrl = (() => {
  try {
    return new URLSearchParams(location.search).get("u") ?? "";
  } catch {
    return "";
  }
})();

function send(
  channel:
    | "shell.popover_close"
    | "shell.popover_refresh"
    | "shell.popover_open_as_tab",
) {
  void bridge.send(channel).catch(() => undefined);
}

function IconBtn({
  title,
  onClick,
  children,
}: {
  title: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      className="flex h-7 w-7 items-center justify-center rounded-md bg-transparent text-cronymax-fg hover:bg-cronymax-surface-2 hover:text-white active:bg-cronymax-border"
    >
      {children}
    </button>
  );
}

export function App() {
  const [url, setUrl] = useState(initialUrl);
  useBridgeEvent("popover.url_changed", (p) => setUrl(p.url ?? ""));

  // Sync title for accessibility/tests.
  useEffect(() => {
    document.title = url ? `popover · ${url}` : "popover";
  }, [url]);

  const secure = url.startsWith("https://");
  const schemeIcon = secure ? "🔒" : url ? "🌐" : "•";

  return (
    <div className="flex h-full items-center gap-1.5 border-b border-cronymax-border bg-cronymax-surface px-2.5">
      <div className="flex min-w-0 flex-1 items-center gap-2 rounded-pill border border-cronymax-border bg-cronymax-bg px-3 py-1.5">
        <span
          className={
            "flex-none text-[11px] " +
            (secure ? "text-cronymax-success" : "text-cronymax-fg-muted")
          }
        >
          {schemeIcon}
        </span>
        <div
          title={url}
          className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[11.5px] text-cronymax-fg"
        >
          {url}
        </div>
      </div>
      <div className="flex flex-none items-center gap-0.5">
        <IconBtn title="Refresh" onClick={() => send("shell.popover_refresh")}>
          <svg
            viewBox="0 0 24 24"
            className="h-3.5 w-3.5 fill-none stroke-current stroke-[1.8] [stroke-linecap:round] [stroke-linejoin:round]"
          >
            <path d="M21 12a9 9 0 1 1-3.2-6.9" />
            <path d="M21 4v5h-5" />
          </svg>
        </IconBtn>
        <IconBtn
          title="Open as tab"
          onClick={() => send("shell.popover_open_as_tab")}
        >
          <svg
            viewBox="0 0 24 24"
            className="h-3.5 w-3.5 fill-none stroke-current stroke-[1.8] [stroke-linecap:round] [stroke-linejoin:round]"
          >
            <path d="M14 4h6v6" />
            <path d="M20 4l-9 9" />
            <path d="M20 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h5" />
          </svg>
        </IconBtn>
        <span className="mx-1 h-[18px] w-px bg-cronymax-border" />
        <IconBtn
          title="Close popover"
          onClick={() => send("shell.popover_close")}
        >
          <svg
            viewBox="0 0 24 24"
            className="h-3.5 w-3.5 fill-none stroke-current stroke-[1.8] [stroke-linecap:round] [stroke-linejoin:round]"
          >
            <path d="M6 6l12 12" />
            <path d="M18 6l-12 12" />
          </svg>
        </IconBtn>
      </div>
    </div>
  );
}
