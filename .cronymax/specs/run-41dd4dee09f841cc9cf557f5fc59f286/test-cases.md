---
title: Test Cases & Coverage Plan
doc_type: test-cases
---

# test-cases

## Test Suite Overview

This project’s primary automated tests target the web/frontend (TypeScript/Vitest), and cover chat store state, localStorage migrations, runtime bridge payloads, Markdown block ID handling, and content stream logic.

The test cases below are derived from exhaustive reading of all `web/test/*.test.ts` code, and are mapped to acceptance criteria and key edge cases.

## Test Cases

### 1. Chat Store Logic
- **stripAnsi function**
  - Strips all CSI, OSC sequences—including 133 markers
  - Converts CR to LF
  - Removes BEL chars
  - Handles text with or without ANSI codes
- **clearPinnedComments**
  - Removes all 'comment' kind attachments
  - Sets `pinnedToPrompt=false` for all comments
- **Prompt Mode Detection**
  - Mode is `"shell"` for `$` (and `$` with leading space)
  - Mode is `"command"` for `/`
  - Mode defaults to `"chat"` for all else

### 2. Chat Content Stream
- **rehydrateContentStream**
  - Matches toolCallId to restore result
  - Handles miss (no match)
  - Handles empty input
  - Does not overwrite non-null results
  - Leaves text/thinking segments unchanged
- **stripContentStreamResults**
  - Nulls only tool_call results, leaves other fields/data unchanged
- **Store reducers**
  - Appending a tool_call segment as running
  - Updating tool_call to done/error/mixed states by id
  - Appending thinking segments, including handling of sealed/unsealed/empty cases

### 3. Chat Store Migration
- **loadChatData**
  - Loads from v4 exclusively if present
  - Migrates v3→v4: reconstructs contentStream, writes v4, deletes v3
  - Migrates v2→v4: strips legacy fields (traceContent), injects traceEntries
  - Returns empty if both v2/v3/v4 missing
- **persistChatData**
  - Writes to v4 only
  - Strips `rawBuf` from shell blocks
  - Strips tool_call results in contentStream

### 4. Runtime Bridge Event & Channel Schemas
- **agent.run schema parsing**
  - Accepts `{task: string}`, rejects plain string
- **events.subscribe schema parsing**
  - Accepts `{run_id}` or `{flow_id}`
  - Ensures `ok: boolean` in response
- **event payload shapes**
  - Accepts: token, run_status (all valid values), log
  - Rejects: unknown kind

### 5. Markdown BlockID Handling
- **Golden round-trip**
  - `assignMissingBlockIds` is identity on already-marked
  - `parseBlockComments` recovers all markers (well-formed block IDs)
  - `stripBlockComments` removes _only_ marker lines; is idempotent
- **Raw input handling**
  - Inserts block marker above every unmarked heading
  - Idempotency: no double-insert on re-run
  - Handles code fences: does not insert multiple markers in fenced blocks

## Coverage Goals

- **Functional/UI state:** All store logic and reducer code paths, including main and migration flows, are exercised.
- **Persistence:** All supported localStorage key formats (v2–v4), error handling, and data rehydration flows are tested.
- **Data schemas:** All runtime bridge payload/event types, happy path and rejection.
- **Text/Markdown utilities:** Exhaustively check that all block ID and block comment mutations maintain data round-trip, idempotency, and proper regex handling for block IDs.
- **Edge cases:** Confirm null/empty/block-missing flows, invalid event payloads, unmarked inputs, old and new data, and critical error cases.

---

**All critical, edge, and acceptance-path behaviors are represented by an automated test in the codebase. The intent is to provide near-total coverage for failures during data migration, persistence, bridge schema evolution, and text block identification.**
