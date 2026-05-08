import { useEffect, useRef, useState } from "react";
import { browser } from "@/shells/bridge";
import { Icon } from "@/components/Icon";

export function App() {
  const [url, setUrl] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Receive URL updates pushed from C++ when the content view navigates.
  useEffect(() => {
    return browser.on("popover_chrome.url_changed", (payload) => {
      setUrl((payload as { url: string }).url);
    });
  }, []);

  function navigate() {
    let target = url.trim();
    if (!target) return;
    if (!/^[a-zA-Z][a-zA-Z0-9+\-.]*:\/\//.test(target)) {
      target = "https://" + target;
    }
    browser.send("shell.popover_navigate", { url: target });
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter") navigate();
    if (e.key === "Escape") inputRef.current?.blur();
  }

  return (
    // Toolbar background uses the theme float surface so it matches the
    // rounded card below it regardless of light/dark mode.
    <div
      className="flex h-full w-full items-center gap-1 px-2"
      style={{ background: "var(--color-cronymax-float)" }}
    >
      {/* URL input — no standalone background; blends into the toolbar */}
      <input
        ref={inputRef}
        type="text"
        value={url}
        onChange={(e) => setUrl(e.target.value)}
        onKeyDown={handleKeyDown}
        onFocus={(e) => e.currentTarget.select()}
        spellCheck={false}
        className="min-w-0 flex-1 rounded px-2 py-1 text-xs outline-none"
        style={{
          background: "transparent",
          color: "var(--color-cronymax-title)",
          border: "none",
        }}
      />

      {/* Action buttons — 32×32 hit area, rounded hover highlight */}
      {(
        [
          { icon: "refresh", event: "shell.popover_refresh", title: "Reload" },
          {
            icon: "link-external",
            event: "shell.popover_open_as_tab",
            title: "Open as tab",
          },
          { icon: "close", event: "shell.popover_close", title: "Close" },
        ] as const
      ).map(({ icon, event, title }) => (
        <button
          key={event}
          onClick={() => browser.send(event, {})}
          title={title}
          aria-label={title}
          className="
            shrink-0 flex h-8 w-8 items-center justify-center rounded
            transition-colors duration-100
            hover:bg-[color:var(--color-cronymax-hover)]
            active:bg-[color:var(--color-cronymax-pressed)]
          "
          style={{
            color: "var(--color-cronymax-caption)",
            background: "transparent",
            border: "none",
            cursor: "pointer",
          }}
        >
          <Icon name={icon} />
        </button>
      ))}
    </div>
  );
}
