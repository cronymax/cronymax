# Flows: a five-minute quickstart

A **Flow** in cronymax is a small graph of typed Agents that produce typed
Documents and review each other's work. This walkthrough builds the smallest
useful Flow — Product → Architect — by hand, then runs it.

## 1. Pick or create a Space

A Space is a workspace folder; cronymax stores every Flow's state under
`<space>/.cronymax/flows/`. Open the app and create a new Space, or reuse
an existing one.

## 2. Drop in the example assets

Copy the bundled example into the Space's `.cronymax/` directory:

```bash
cp -R assets/examples/agents      <space>/.cronymax/agents/
cp -R assets/examples/doc_types   <space>/.cronymax/doc_types/
cp -R assets/examples/flows       <space>/.cronymax/flows/
```

You should now have:

```
.cronymax/
  agents/{product,architect,critic}.agent.yaml
  doc_types/{prd,tech-spec}.doc-type.yaml
  flows/simple-prd-to-spec/flow.yaml
```

cronymax watches these directories — registries reload automatically on
save (see `flow/fs_watcher.cc`).

## 3. Start a Run

Open the **Flows** sidebar and pick **simple-prd-to-spec**, then click
**Start Run**. The chat panel (`web/flow/chat.html`) opens with the new
Run's id in the URL (`?flow=simple-prd-to-spec&run=r-<ms>-<hex>`).

The first agent declared in the flow (`product`) starts immediately. As it
emits `submit_document`, you'll see a **document card** in the chat with
**Approve** and **Request changes** buttons (the Flow declares
`requires_human_approval: true` on the `prd` edge). Click **Approve** to
let the next agent (`architect`) consume the PRD.

## 4. Watch the trace

Every state change is appended to `<run-dir>/trace.jsonl` — open it in a
terminal to watch agents start, tools fire, and reviewers verdict:

```bash
tail -f .cronymax/flows/simple-prd-to-spec/runs/<run-id>/trace.jsonl
```

The same stream powers the chat panel via the `event.subscribe` bridge
channel (replay-then-live: late subscribers see history first).

## 5. Iterate by editing YAML

Edit `architect.agent.yaml` to tighten the system prompt; the watcher
reloads the AgentRegistry on the next file write. New Runs pick up the
change immediately. Paused Runs (e.g. waiting on Approve) survive an app
restart — the FlowRuntime rehydrates state from `state.json`.

## Beyond the basics

- `bug-fix-loop/flow.yaml` shows `@mention` backward routing: the Coder
  pings `@product` mid-patch when requirements are unclear.
- `max_review_rounds` + `on_review_exhausted: approve|halt` controls what
  happens when the reviewer never says "approve".
- See [`docs/multi_agent_orchestration.md`](multi_agent_orchestration.md)
  for the full schema reference.
