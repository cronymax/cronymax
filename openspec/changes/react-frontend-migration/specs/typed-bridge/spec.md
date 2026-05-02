## ADDED Requirements

### Requirement: Discriminated-union channel registry

The frontend SHALL declare every bridge channel in a single TypeScript registry that maps each channel name to a request payload schema and a response payload schema. The registry SHALL be the single source of truth for channel shape on the JS side.

#### Scenario: New channel is added to the registry

- **WHEN** a new bridge channel `tool.foo` is introduced
- **THEN** an entry `"tool.foo": { req: …, res: … }` is added to `web/src/shared/bridge_channels.ts` before any panel calls `bridge.send("tool.foo", …)`

#### Scenario: Unknown channel is rejected at compile time

- **WHEN** a developer writes `bridge.send("nonexistent.channel", …)`
- **THEN** TypeScript reports an error indicating the channel name is not in the registry

---

### Requirement: Runtime payload validation via Zod

The bridge SHALL validate request payloads against the channel's `req` schema before serializing to JSON, and SHALL validate response payloads against the channel's `res` schema before resolving the caller. Inbound broadcast event payloads delivered via `useBridgeEvent` SHALL be validated against the channel's payload schema before invoking the handler.

#### Scenario: Outbound payload mismatch fails fast

- **WHEN** a panel calls `bridge.send("space.create", { name: "x" })` (missing required `root_path`)
- **THEN** the bridge throws a Zod validation error before any IPC traffic occurs

#### Scenario: Inbound payload mismatch surfaces a clear error

- **WHEN** C++ broadcasts `terminal.created` with a payload missing the `id` field
- **THEN** `useBridgeEvent("terminal.created", …)` does not invoke its handler with malformed data; instead the dev-mode error overlay displays the Zod validation error

#### Scenario: Streaming channels may opt out of full validation

- **WHEN** a high-frequency channel (e.g., `terminal.output`, `agent.llm.stream`) is registered with the fast-path flag
- **THEN** the bridge skips full Zod validation for that channel's events and the channel registry documents the rationale

---

### Requirement: Typed `bridge.send` and `bridge.on`

The bridge SHALL expose `bridge.send<C extends Channel>(channel: C, payload?: PayloadOf<C>): Promise<ResponseOf<C>>` and `bridge.on<C extends Channel>(channel: C, handler: (payload: PayloadOf<C>) => void): UnsubscribeFn`, where the payload and response types are inferred from the channel registry.

#### Scenario: Caller gets type-safe response

- **WHEN** a panel writes `const t = await bridge.send("terminal.new")`
- **THEN** TypeScript infers `t` as `{ id: string; name: string }` (the schema in the registry) without manual casting

#### Scenario: Subscriber gets type-safe payload

- **WHEN** a panel writes `bridge.on("terminal.created", (t) => …)`
- **THEN** TypeScript infers `t` as the registry-declared payload type

---

### Requirement: `useBridgeEvent` hook with auto-cleanup

The frontend SHALL provide a `useBridgeEvent<C>(channel, handler)` React hook that subscribes to the channel on mount, unsubscribes on unmount, and validates inbound payloads against the channel registry before invoking `handler`. The hook SHALL not invoke `handler` after the component has unmounted.

#### Scenario: Hook unsubscribes on unmount

- **WHEN** a component using `useBridgeEvent("terminal.created", h)` unmounts and a `terminal.created` event subsequently arrives
- **THEN** the handler `h` is not invoked

---

### Requirement: `useBridgeQuery` hook for request/response

The frontend SHALL provide a `useBridgeQuery<C>(channel, payload?)` React hook that returns `{ data, error, loading, send }`. The hook SHALL call `bridge.send(channel, payload)` on mount (and when `payload` changes if provided), expose the resolved value as `data`, and expose any thrown error (including Zod validation errors) as `error`.

#### Scenario: Query reflects loading and resolved states

- **WHEN** a component mounts with `useBridgeQuery("space.list")`
- **THEN** the returned object first reports `{ loading: true, data: undefined, error: undefined }`, then `{ loading: false, data: [...], error: undefined }` once the response arrives

---

### Requirement: Optimistic update pattern documented

The bridge layer SHALL document the optimistic-update pattern for create-style flows: dispatch an optimistic action with a temporary id, await `bridge.send`, dispatch a commit action with the real id on success, dispatch a revert action on failure. This pattern SHALL replace today's reliance on broadcast events to populate local state after a `bridge.send`.

#### Scenario: Sidebar terminal-creation no longer relies on broadcast

- **WHEN** the user clicks "New Terminal" in the migrated sidebar
- **THEN** the sidebar dispatches an optimistic `termAddingPlaceholder` action immediately, awaits `bridge.send("terminal.new")`, dispatches `termCreated` with the returned `{id, name}` on success, and the row appears regardless of whether the `terminal.created` broadcast event arrives before, after, or not at all
