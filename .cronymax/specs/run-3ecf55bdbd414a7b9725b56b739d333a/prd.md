---
title: runtime/handler.rs — Architectural Split PRD
doc_type: prd
---

# PRD: `runtime/handler.rs` — Architectural Split

## Goal

Decompose the monolithic `crates/cronymax/src/runtime/handler.rs` (~1,550 lines) into a coherent directory of focused, single-responsibility modules. The result must be a **pure mechanical refactor**: identical observable behaviour, identical public API, zero new traits or abstractions — only improved navigability, parallel editability, and testability.

---

## Users

| Persona | Context |
|---|---|
| **Feature engineers** | Add new `ControlRequest` variants or extend existing handler logic. Currently must scan ~1,550 lines to find the relevant arm. |
| **Reviewers / PR authors** | Today a change to flow-review logic produces a multi-hundred-line diff mixed with unrelated code. Post-split, a review-only PR touches one ~200-line file. |
| **QA / test authors** | Want to add focused unit tests next to specific handler logic. Currently there is no natural co-location point. |
| **New contributors / onboarders** | Need to understand "where does `FlowRunApprove` live?" today requires reading the entire file; post-split the answer is `review_ops.rs`. |

---

## User Stories

1. **As a feature engineer**, I want to find any `ControlRequest` handler in ≤ 10 seconds so that I can make targeted changes without reading irrelevant code.
2. **As a feature engineer**, I want to edit flow-review logic and run-start logic in the same PR without creating merge conflicts with a colleague editing registry or terminal logic simultaneously.
3. **As a reviewer**, I want a PR that touches only flow-review behaviour to show a diff bounded to `review_ops.rs` (~200 lines) so that I can review it thoroughly in one sitting.
4. **As a test author**, I want each sub-module to have a natural `#[cfg(test)]` block co-located with its handlers so that unit tests are easy to add and find.
5. **As a new contributor**, I want `mod.rs` to serve as an authoritative map of all handler responsibilities so that I can understand the system at a glance without reading implementation details.
6. **As a Rust compiler**, every existing compilation unit, public API surface, and integration test must continue to pass without modification after the split.

---

## Acceptance Criteria

### AC-1 — Module structure
The file `handler.rs` is replaced by a directory `handler/` containing **exactly** the following files (no more, no fewer for this slice):

| File | Primary contents |
|---|---|
| `mod.rs` | `RuntimeHandler` struct + all constructors + `Handler` trait impl (thin routing `match` only) + `run_op` helper |
| `helpers.rs` | All pure, stateless utility functions (≥ the 8 listed in the prototype) |
| `subscription.rs` | `handle_subscribe`, `handle_unsubscribe`, fan-out helpers |
| `run_start.rs` | `handle_start_run`, `LlmParams`, `spawn_supervision_task`, `spawn_chat_supervision_task` |
| `run_ops.rs` | `handle_resume_run`, `handle_cancel_run`, `handle_pause_run`, `handle_post_input`, `handle_resolve_review`, `handle_swap_memory` |
| `workspace_ops.rs` | `handle_workspace_layout`, `handle_file_read`, `handle_file_write` |
| `flow_ops.rs` | `handle_flow_list`, `handle_flow_load`, `handle_flow_save` |
| `registry_ops.rs` | All eight agent-registry and doc-type registry handlers |
| `terminal_ops.rs` | `handle_terminal_start/input/resize/stop` |
| `document_ops.rs` | `handle_document_list/read/submit/suggestion_apply`, `handle_mention_parse` |
| `review_ops.rs` | `handle_flow_run_get_pending_reviews`, `handle_get_session_pending_actions`, `handle_flow_run_approve`, `handle_flow_run_request_changes` |
| `session_ops.rs` | `handle_get_space_snapshot`, `handle_session_list`, `handle_session_thread_inspect`, `handle_list_provider_models` |

### AC-2 — `mod.rs` line budget
`mod.rs` contains **≤ 250 lines** (including doc comments). The `handle_control` `match` body in `mod.rs` consists exclusively of one-liner delegating arms.

### AC-3 — No logic changes
A diff of any sub-handler extracted into its own file must show **no semantic differences** from the corresponding code in the original `handler.rs`. Pure moves are the only permitted change in phases 1–7.

### AC-4 — Compilation and tests pass
`cargo build` and `cargo test` (all existing tests, including integration tests) pass without modification on the final commit. No test files are changed.

### AC-5 — Visibility
All sub-handler methods are declared `pub(super)`. Helper functions in `helpers.rs` are `pub(super)`. No new items are made `pub` beyond what was `pub` in the original file.

### AC-6 — Sub-module internal items remain private
Private structs (`LlmParams`) and private functions (`spawn_supervision_task`) introduced during the extraction of `run_start.rs` are scoped to `run_start.rs` only (no `pub(super)` or wider unless previously public).

### AC-7 — Phased PR delivery
The split is delivered in **7 sequential PRs** matching the migration phases defined in the approved prototype. Each individual PR must pass `cargo build` and `cargo test` before the next is opened.

| Phase | Scope |
|---|---|
| 1 | Rename `handler.rs` → `handler/mod.rs`; CI must be green (zero changes) |
| 2 | Extract `helpers.rs` |
| 3 | Extract `subscription.rs`, `workspace_ops.rs`, `flow_ops.rs`, `registry_ops.rs`, `terminal_ops.rs`, `document_ops.rs`, `session_ops.rs` |
| 4 | Extract `run_ops.rs` |
| 5 | Extract `review_ops.rs` |
| 6 | Extract `run_start.rs` (apply `LlmParams` + supervision task extractions) |
| 7 | Thin `mod.rs` to routing table + struct/constructors only; verify line budget AC-2 |

### AC-8 — No new runtime dependencies
The Cargo.toml for the `cronymax` crate must not gain any new dependencies as a result of this refactor.

### AC-9 — Doc comments on each file
Every new `.rs` file in `handler/` begins with a module-level `//!` doc comment (≥ 1 sentence) describing its single responsibility.

---

## Non-Goals

- **No new traits or abstraction layers.** All methods remain `impl RuntimeHandler` blocks. Introducing a `SubHandler` trait or similar is explicitly out of scope.
- **No logic changes to `handle_start_run`.** The ~480-line handler is extracted as-is. A follow-up `supervision.rs` extraction is noted but deferred.
- **No changes to the `ControlRequest` or `ControlResponse` enums.**
- **No test additions in this slice.** Test co-location infrastructure is enabled by this refactor but writing new unit tests is a separate story.
- **No renaming of public symbols.** `RuntimeHandler`, `Handler`, all constructors, and all public methods keep their exact names.
- **No migration of any other file** in `crates/cronymax/src/runtime/` beyond `handler.rs`.
