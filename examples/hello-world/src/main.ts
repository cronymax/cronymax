import type { ExtensionContext } from "@cronymax/extension";
import { commands, window } from "@cronymax/extension";

export async function activate(ctx: ExtensionContext): Promise<void> {
  const log = window.createOutputChannel("Hello World", { log: true });
  ctx.subscriptions.push(log);

  log.info("hello-world activate()");

  ctx.subscriptions.push(
    commands.register("cronymax-examples.hello-world.hello", async () => {
      log.info("hello command invoked");
      await window.showInformationMessage("Hello from cronymax!");
    }),
  );
}

export async function deactivate(): Promise<void> {
  // Nothing extra — ctx.subscriptions[].dispose() is called by the host.
}
