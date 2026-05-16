import { useEffect, useRef, useState } from "react";
import { Icon } from "@/components/Icon";
import { Button } from "@/components/ui/button";
import { browser, shells } from "@/shells/bridge";

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
      target = `https://${target}`;
    }
    shells.browser.shell.popover_navigate({ url: target });
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter") navigate();
    if (e.key === "Escape") inputRef.current?.blur();
  }

  return (
    <div className="flex h-full w-full items-center gap-1 bg-card px-2">
      {/* URL input — no standalone background; blends into the toolbar */}
      <input
        ref={inputRef}
        type="text"
        value={url}
        onChange={(e) => setUrl(e.target.value)}
        onKeyDown={handleKeyDown}
        onFocus={(e) => e.currentTarget.select()}
        spellCheck={false}
        className="min-w-0 flex-1 rounded bg-transparent px-2 py-1 text-xs text-foreground outline-none"
      />

      {/* Action buttons — 32×32 hit area, rounded hover highlight */}
      {(
        [
          ["refresh", "browser.shell.popover_refresh", "Reload"],
          ["link-external", "browser.shell.popover_open_as_tab", "Open as tab"],
          ["close", "browser.shell.popover_close", "Close"],
        ] as const
      ).map(([icon, event, title]) => (
        <Button
          key={event}
          variant="ghost"
          size="icon"
          className="h-8 w-8 shrink-0 text-muted-foreground"
          onClick={() => browser.send(event, {})}
          title={title}
          aria-label={title}
        >
          <Icon name={icon} />
        </Button>
      ))}
    </div>
  );
}
