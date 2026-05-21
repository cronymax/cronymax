---
title: Multi-Turn Chat History Bug — Fix Prototype
doc_type: prototype
---

# Multi-Turn Chat History Bug — Fix Prototype

## Problem Statement

In the Cronymax chat tab, the LLM loses awareness of prior conversation turns after the first exchange. A user can ask a follow-up question that clearly references what was just discussed and the assistant responds as if the conversation just started.

---

## Root Cause (Diagnosed)

The bug lives in **`crates/cronymax/src/runtime/handler/run_start.rs`** in the post-run history flush.

After each chat turn completes, the code does:

```rust
if let Some(thread) = authority.session_thread(sid) {
    let store = ChatStore::new(cache_dir);
    let _ = store.append_turns(sid, &thread);   // ← BUG: full thread, not just new messages
}
```

`authority.session_thread(sid)` returns the **complete** accumulated conversation (old + new). `append_turns` then **appends the whole thing** to `history.jsonl`, which already contains all the old messages from prior turns.

### What `history.jsonl` looks like after 2 turns

```
Turn 1 append → [system, user₁, assistant₁]
Turn 2 append → [system, user₁, assistant₁, system, user₂, assistant₂]  ← old messages duplicated!
```

After turn 2, `history.jsonl` contains:
```
system
user₁
assistant₁
system          ← DUPLICATE
user₁           ← DUPLICATE
assistant₁      ← DUPLICATE
user₂
assistant₂
```

When turn 3 loads this via `store.load_history()`, the `prior_thread` contains the duplicated messages. Crucially, **a `system` message now appears mid-conversation**, which:
- Anthropic's API rejects or silently truncates
- OpenAI-compat providers may silently ignore context before the second system message
- Either way, the LLM behaves as if turns 1–2 never happened

This contrasts with `agent_runner.rs` (`spawn_chat`), which correctly appends **only the delta**:
```rust
let _ = store.append_turns(&sid, &updated_thread[prior_thread_len..]);  // ← correct
```

---

## Desired User Experience (Post-Fix)

### Scenario: 3-turn conversation

| Turn | User says | Expected LLM behaviour |
|------|-----------|------------------------|
| 1 | "My name is Alice, remember that." | "Got it, Alice!" |
| 2 | "What's my name?" | "Your name is Alice." ✓ |
| 3 | "What did we talk about?" | "You told me your name is Alice." ✓ |

Currently, turn 2 and beyond respond as if the conversation is fresh:
> "I don't have information about your name."

### User Flow (unchanged UI — internal fix only)

```
User opens Chat Tab
       │
       ▼
  Types message → [Send]
       │
       ▼
  LLM responds with full conversation context ← FIXED (was broken from turn 2+)
       │
       ▼
  User types follow-up → [Send]
       │
       ▼
  LLM responds aware of all prior turns ← FIXED
```

No UI changes are needed. The fix is entirely in the persistence layer.

---

## Proposed Fix Design

### Primary Fix — Append Delta Only (`run_start.rs`)

Before the `LoopConfig` is constructed, capture the length of the effective thread:

```
effective_thread_len = effective_thread.len()
```

In the post-run flush, replace the current full-append with a delta-append:

```
new_messages = thread[effective_thread_len..]
append_turns(sid, new_messages)              // only the new turns
```

This mirrors the already-correct pattern in `agent_runner.rs`.

### Secondary Fix — Compaction Rewrite (`run_start.rs`)

When context-window compaction fires (`result.compacted == true`), the in-memory thread is shorter than what's on disk. Simply appending the delta on top of the stale file would leave stale old messages. In this case the file must be **fully rewritten**:

```
if compacted:
    store.rewrite_history(sid, thread)       // truncate + rewrite
else:
    store.append_turns(sid, thread[effective_thread_len..])
```

This requires adding a `rewrite_history` method to `ChatStore` that atomically replaces `history.jsonl` (write to `.tmp`, rename — same pattern used by `write_meta`).

### Scope Summary

| Component | Change | Risk |
|-----------|--------|------|
| `run_start.rs` | Capture `effective_thread_len`; use delta-append | Low — trivial slice |
| `run_start.rs` | Rewrite file on compaction | Low — mirrors `write_meta` pattern |
| `chat_store.rs` | Add `rewrite_history(sid, turns)` method | Low — new public method |
| `run_ops.rs` (ResumeRun) | Same delta-append fix needed | Low |

---

## Acceptance Criteria (Preview)

1. After 5 consecutive turns in a single chat tab, the LLM correctly recalls facts stated in turn 1.
2. `history.jsonl` contains no duplicate `system`, `user`, or `assistant` entries across turns.
3. After a compaction event, `history.jsonl` reflects the compacted + new messages (not old uncompacted + new).
4. The fix has zero effect on flow-run invocations (already guarded by `maybe_flow_ctx.is_none()`).
