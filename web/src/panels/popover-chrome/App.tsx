import { useEffect, useRef, useState } from "react";
import { bridge } from "@/bridge";

export function App() {
  const [url, setUrl] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Receive URL updates pushed from C++ when the content view navigates.
  useEffect(() => {
    return bridge.on("popover_chrome.url_changed", (payload) => {
      setUrl((payload as { url: string }).url);
    });
  }, []);

  function navigate() {
    let target = url.trim();
    if (!target) return;
    // Prepend https:// if no scheme present.
    if (!/^[a-zA-Z][a-zA-Z0-9+\-.]*:\/\//.test(target)) {
      target = "https://" + target;
    }
    bridge.send("shell.popover_navigate", { url: target });
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter") navigate();
    if (e.key === "Escape") inputRef.current?.blur();
  }

  return (
    <div
      className="flex h-full w-full items-center gap-2 px-3"
      style={{ background: "#1C1E23" }}
    >
      {/* URL input — flex fill */}
      <input
        ref={inputRef}
        type="text"
        value={url}
        onChange={(e) => setUrl(e.target.value)}
        onKeyDown={handleKeyDown}
        onFocus={(e) => e.currentTarget.select()}
        spellCheck={false}
        className="min-w-0 flex-1 rounded px-3 py-1 text-xs outline-none"
        style={{
          background: "#15171B",
          color: "#E8E8EA",
          border: "none",
        }}
      />

      {/* Reload */}
      <button
        onClick={() => bridge.send("shell.popover_refresh", {})}
        title="Reload"
        className="shrink-0 text-sm leading-none"
        style={{
          color: "#9AA0A8",
          background: "transparent",
          border: "none",
          cursor: "pointer",
        }}
      >
        ↻
      </button>

      {/* Open as tab */}
      <button
        onClick={() => bridge.send("shell.popover_open_as_tab", {})}
        title="Open as tab"
        className="shrink-0 text-sm leading-none"
        style={{
          color: "#9AA0A8",
          background: "transparent",
          border: "none",
          cursor: "pointer",
        }}
      >
        ↗
      </button>

      {/* Close popover */}
      <button
        onClick={() => bridge.send("shell.popover_close", {})}
        title="Close"
        className="shrink-0 text-sm leading-none"
        style={{
          color: "#9AA0A8",
          background: "transparent",
          border: "none",
          cursor: "pointer",
        }}
      >
        ✕
      </button>
    </div>
  );
}
