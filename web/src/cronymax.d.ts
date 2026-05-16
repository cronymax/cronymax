/**
 * Global type declarations for the `window.cronymax` object injected by
 * the C++ host (App::OnContextCreated).  Placing them here (ambient .d.ts)
 * makes the types available in every TypeScript module without any import.
 */

declare global {
  interface Window {
    cefQuery?: (opts: {
      request: string;
      onSuccess: (response: string) => void;
      onFailure: (errorCode: number, errorMessage: string) => void;
      persistent?: boolean;
    }) => number;
    cefQueryCancel?: (queryId: number) => void;

    cronymax?: {
      /** Renderer ↔ browser-process IPC (cefQuery + dispatch). */
      browser?: {
        /**
         * Binary msgpack fast path injected by App::OnContextCreated (C++).
         * Sends a binary-framed request to the browser process and returns a
         * Promise resolving with the decoded response object.
         */
        send?: (channel: string, payload: unknown) => Promise<unknown>;

        /** Set by bridge.ts; called by C++ (bridge_handler.cc / main_window.cc). */
        on?: (event: string, payload: unknown) => void;
      };
      /** Renderer ↔ Rust runtime IPC via CEF process messages (injected by App::OnContextCreated). */
      runtime?: {
        /**
         * Send a ControlRequest to the Rust runtime via CEF process message.
         * `request` must be a ControlRequest-shaped object (with a `kind` field).
         * Returns a Promise resolving with the decoded ControlResponse object.
         */
        send(request: Record<string, unknown>): Promise<unknown>;

        /**
         * Called by C++ (renderer app.cc) when a kMsgRuntimeEvent arrives.
         * bridge.ts assigns this to route events to JS subscribers.
         * Args: (subId: subscription UUID, event: decoded inner event object).
         */
        on?: (subId: string, event: unknown) => void;
      };
    };
  }
}

export {};
