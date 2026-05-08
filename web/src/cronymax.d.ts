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
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        send: (channel: any, payload?: any) => Promise<any>;
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        on: (event: any, handler: (payload: any) => void) => () => void;
        /** Aliased from window.cefQuery by App::OnContextCreated (C++). */
        query?: (opts: {
          request: string;
          onSuccess: (response: string) => void;
          onFailure: (errorCode: number, errorMessage: string) => void;
          persistent?: boolean;
        }) => number;
        /** Aliased from window.cefQueryCancel by App::OnContextCreated (C++). */
        queryCancel?: (queryId: number) => void;
        /** Set by bridge.ts; called by C++ (bridge_handler.cc / main_window.cc). */
        onDispatch?: (event: string, payload: unknown) => void;
      };
      /** Renderer ↔ Rust runtime IPC via CEF process messages (injected by App::OnContextCreated). */
      runtime?: {
        /**
         * Send a ControlRequest to the Rust runtime via CEF process message.
         * `request` must be a ControlRequest-shaped object (with a `kind` field).
         * Returns a Promise resolving with the ControlResponse JSON string.
         */
        send(request: Record<string, unknown>): Promise<string>;
        /**
         * Subscribe to a runtime topic via CEF process message.
         * The callback receives the inner event object JSON string.
         * Returns an unsubscribe function.
         */
        subscribe(topic: string, callback: (event: string) => void): () => void;
      };
    };
  }
}

export {};
