---
title: Layout Migrator, Cronymax Authority & FFI — Test Cases
doc_type: test-cases
---

## Test Suite Overview

This test suite covers the migration, creation, and runtime behaviors of the Cronymax agent, runtime, and protocol boundary layers. The goal is to verify correctness of:
- Layout migration and persistence logic (`LayoutMigrator` for v0-v4).
- Authority, agent, and session state creation and access.
- Runtime and protocol integration (including FFI, GIPS, and error handling).
- Compaction, workspace, version, and protocol boundary error conditions.

We cover happy paths, acceptance scenarios, and edge/failure cases as described in the associated test cases.

---

## Test Cases

### 1. Layout Migration and Persistence (C++: `layout_migrator_test.cc`)
- **Fresh Install, No Old Data**
  - Create new directory, run migrator: expect sentinels for v2/v3/v4, no snapshot created.
- **Migration from V1 Profiles Layout**
  - Prepare V1 layout, run migration: expect old data moved and available as v3/v4.
- **Migration from V0 Flat Layout**
  - Prepare V0 layout, run migration: expect correct migration to v3/v4 final layout.
- **Migration from Double Runtime Legacy Layout**
  - Test legacy double-runtime path migration; assert data ends under proper directory.
- **Migration from V2 to V3/V4**
  - Insert sentinel at v2, verify complete migration and removal of legacy files.
- **Already Fully Migrated/Idempotency**
  - Start with up-to-date layout, run migrator: expect no changes (idempotency).

### 2. Rust Cronymax Runtime Authority, Agent, and Flow (Rust)
- **Session Creation and Handling**
  - Creating new session ID results in empty thread; reflected in snapshot.
- **Thread Persistence**
  - Flushed messages to thread persist and reload correctly.
- **Compaction, Token Estimates, and Summarization**
  - Large threads are compacted; small ones not.
- **Integration: AgentRunner + Mock LLM/Capability**
  - Spawning agent results in authority run, single LLM request.
- **Session Binding in Chat**
  - spawn_chat binds session correctly, resolves immediately.

### 3. End-to-End and FFI Boundary (Rust)
- **Runtime End-to-end: State Persists on Restart**
  - Complete run (start, tool call, review, memory write, complete), restart, assert all state and memory present.
- **Error: Corrupted or Unknown Schema in Persistence**
  - Corrupt file → should fail rehydrate; future schema version refuses to boot.
- **C ABI Boundary: Basic Round-Trip**
  - Start service, connect, send Hello→Welcome and Ping→Pong flow through FFI C ABI.
- **GIPS Protocol: Basic Round-Trip**
  - GIPS handshake (Hello/Welcome) and Ping/Pong round-trip via synchronous endpoint.
- **GIPS Cancel-Safety Regression**
  - Simulate unsolicited server push, then normal ping; check no receiver drop or silent exit.

---

## Coverage Goals

- **Edge Cases**:
  - Legacy layouts, corrupted snapshots, schema mismatches, idempotency of migrations.
- **Happy Paths**:
  - Installing/running fresh, normal agent and chat session flows, persistence through restart, protocol boundary Hello/Ping.
- **Acceptance Criteria**:
  - All run lifecycles, compaction, agent and session logic, protocol flows, and migrations behave as specified. No silent errors on disk corruption or future schema.
  - Agent spawning and chat invocation must not touch external resource (LLM/cap) unless intended.

**This suite, spanning both C++ and Rust, ensures that migration, authority, session, runtime, FFI, GIPS, and workspace management logic are robust, deterministic, and ready for integration QA.**
