---
title: Multi-Turn Chat History Bug Fix — PRD
doc_type: prd
---

# Multi-Turn Chat History Bug Fix — PRD

## Goal

Eliminate the chat-context amnesia bug in the Cronymax chat tab where the LLM loses awareness of prior conversation turns after the first exchange. The fix ensures that `history.jsonl` is append-correct at all times — containing each message exactly once — so every LLM invocation receives a well-formed, non-duplicated conversation thread.

---

## Users

**Primary:** All Cronymax users who engage in multi-turn chat sessions via the Chat Tab. This includes developers using the app as a coding assistant and knowledge workers using it for iterative Q&A.

**Secondary:** Automated flow-run agents that share the same persistence layer (these are already working correctly and must not be regressed).

---

## User Stories

1. **As a chat user**, I want the LLM to remember facts I shared in earlier turns of our conversation so that I do not have to repeat context on every message.

2. **As a chat user**, I want follow-up questions to be answered in the context of the full conversation so that the dialogue feels coherent and productive.

3. **As a chat user who has a long conversation**, I want the app to handle context-window compaction gracefully so that the conversation remains coherent even after the LLM's context window is trimmed.

4. **As a developer running automated flows**, I want the bug fix to be scoped entirely to the chat-tab code path so that existing flow-run behaviour is unchanged.

---

## Acceptance Criteria

### AC-1 — Delta-only append in `run_start.rs`
- Before constructing the `LoopConfig`, the code captures `effective_thread_len = effective_thread.len()`.
- After the run completes, only messages at indices `[effective_thread_len..]` of the updated thread are passed to `store.append_turns()`.
- The full thread is **never** re-appended to an already-populated `history.jsonl`.

### AC-2 — Compaction rewrite in `run_start.rs`
- When `result.compacted == true`, `store.rewrite_history(sid, &thread)` is called instead of `append_turns`.
- The rewrite is atomic: write to a `.tmp` file then rename (same pattern as `write_meta`).
- After a compaction event, `history.jsonl` contains exactly the compacted thread plus any new messages from the current turn.

### AC-3 — `rewrite_history` method on `ChatStore`
- `ChatStore` exposes a new `pub fn rewrite_history(sid, turns)` method.
- On success, the existing `history.jsonl` is fully replaced.
- The method follows the same error-handling conventions as `append_turns`.

### AC-4 — Same delta-append fix applied to `ResumeRun` in `run_ops.rs`
- `ResumeRun` also captures `prior_thread_len` before the run loop and uses delta-append after the run, matching the fix in `run_start.rs`.

### AC-5 — No duplicate messages in `history.jsonl`
- After N consecutive chat turns (N ≥ 5), parsing `history.jsonl` as JSONL yields exactly one entry per message role/content/turn — no duplicate `system`, `user`, or `assistant` entries.

### AC-6 — LLM context correctness
- In an end-to-end test with 3+ turns, the LLM correctly recalls a fact introduced in turn 1 when asked about it in turn 2 and turn 3.

### AC-7 — Flow runs unaffected
- All existing flow-run integration tests pass without modification.
- The chat-tab fix is guarded by the existing `maybe_flow_ctx.is_none()` check (or equivalent) so flow invocations are not touched.

### AC-8 — No data loss on crash/restart
- A chat session can be interrupted mid-turn (simulated by killing the process) and then resumed; `history.jsonl` remains parseable and contains no partial writes.

---

## Non-Goals

- **UI changes:** No changes to the chat panel frontend. The fix is entirely in the Rust persistence layer.
- **Flow-run history:** The `agent_runner.rs` / `spawn_chat` path already appends deltas correctly; no changes are required there.
- **LLM provider switching:** This fix does not address provider-specific system-message placement policies; those remain out of scope.
- **History migration:** Existing corrupted `history.jsonl` files from affected sessions will not be automatically repaired. Users with broken sessions will need to start a new session.
- **Context-window management strategy:** The scope covers only the persistence correctness of compaction events, not any changes to when or how compaction is triggered.
- **Performance optimisation:** Rewriting `history.jsonl` on compaction is acceptable for prototype scale; production-grade optimisation (e.g. WAL, SQLite) is out of scope.
