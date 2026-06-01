import type { ExtensionContext, WebviewView } from "@cronymax/extension";
import { window } from "@cronymax/extension";

// The view id must match the `cronymax.ui.sidebar.view` contribution in
// cronymax-extension.json. It's globally namespaced (publisher.name.viewId).
const VIEW_ID = "cronymax-examples.view-messaging.panel";

export async function activate(ctx: ExtensionContext): Promise<void> {
  const log = window.createOutputChannel("View Messaging", { log: true });
  ctx.subscriptions.push(log);
  log.info("view-messaging activate()");

  // Register a provider for the rail view. resolveWebviewView runs each time
  // the platform mounts the view (user clicks the rail icon). The view's
  // iframe loads view/index.html and talks to us over `webview`.
  ctx.subscriptions.push(
    window.registerWebviewViewProvider(VIEW_ID, {
      resolveWebviewView(view: WebviewView): void {
        log.info(`resolveWebviewView(${view.viewId}) visible=${view.visible}`);

        // Echo every message from the iframe back, stamped by the host, so
        // the round-trip is visible in the view.
        view.webview.onDidReceiveMessage((msg) => {
          log.info(`view → ext: ${JSON.stringify(msg)}`);
          void view.webview.postMessage({
            type: "echo",
            from: "extension-host",
            received: msg,
            at: Date.now(),
          });
        });

        view.onDidDispose(() => log.info(`view ${view.viewId} disposed`));
        view.onDidChangeVisibility(() => log.info(`view ${view.viewId} visibility → ${view.visible}`));

        // Push an initial greeting the instant the view resolves — this is
        // the ext → view direction working before the iframe says anything.
        void view.webview.postMessage({
          type: "greeting",
          from: "extension-host",
          text: "Hello from the extension host!",
        });
      },
    }),
  );
}

export async function deactivate(): Promise<void> {
  // ctx.subscriptions[].dispose() (run by the host on deactivate) unregisters
  // the provider and fires the live view's onDidDispose.
}
