// Hand-rolled unit tests for the YAML loaders in cronymax_document and
// cronymax_flow. Run via `cmake --build build --target loader_test &&
// ./build/loader_test`. Exits 0 on success, non-zero on first failure.
//
// Convention matches tools/native_probe.cc: a small CLI binary asserting
// invariants. No external test framework is pulled in for this change.

#include <cassert>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <optional>
#include <string>
#include <thread>

#include "document/agent_definition.h"
#include "document/doc_type_schema.h"
#include "document/document_store.h"
#include "document/review_store.h"
#include "document/reviewer_pipeline.h"
#include "document/reviews_state.h"
#include "document/schema_reviewer.h"
#include "agent/agent_runtime.h"
#include "agent/tool_registry.h"
#include "flow/flow_definition.h"
#include "flow/flow_registry.h"
#include "flow/flow_runtime.h"
#include "flow/mention_parser.h"
#include "flow/router.h"
#include "flow/trace_event.h"
#include "flow/trace_writer.h"
#include "event_bus/app_event.h"
#include "event_bus/event_bus.h"
#include "workspace/space_store.h"

using cronymax::AgentDefinition;
using cronymax::DocTypeSchema;
using cronymax::DocumentStore;
using cronymax::FlowDefinition;
using cronymax::FlowRegistry;
using cronymax::FlowRuntime;
using cronymax::FlowRunStatus;
using cronymax::DocComment;
using cronymax::DocStatus;
using cronymax::ReviewsState;
using cronymax::ReviewStore;
using cronymax::ReviewerPipeline;
using cronymax::ReviewerPolicy;
using cronymax::PipelineOutcome;
using cronymax::LlmReviewer;
using cronymax::ReviewerVerdict;
using cronymax::SchemaReviewer;
using cronymax::AgentRuntime;
using cronymax::AgentIdentity;
using cronymax::FlowBindings;
using cronymax::ToolCall;
using cronymax::ToolResult;

namespace {

int failures = 0;

#define EXPECT_OK(result)                                              \
  do {                                                                 \
    auto&& _r = (result);                                              \
    if (!_r.ok()) {                                                    \
      std::fprintf(stderr, "  FAIL: expected ok, got error: %s\n",     \
                   _r.error().ToString().c_str());                     \
      ++failures;                                                      \
    }                                                                  \
  } while (0)

#define EXPECT_ERR_CONTAINS(result, needle)                                  \
  do {                                                                       \
    auto&& _r = (result);                                                    \
    if (_r.ok()) {                                                           \
      std::fprintf(stderr, "  FAIL: expected error containing \"%s\", "      \
                           "got ok\n",                                       \
                   needle);                                                  \
      ++failures;                                                            \
    } else {                                                                 \
      const auto msg = _r.error().ToString();                                \
      if (msg.find(needle) == std::string::npos) {                           \
        std::fprintf(stderr, "  FAIL: error \"%s\" does not contain \"%s\"\n", \
                     msg.c_str(), needle);                                   \
        ++failures;                                                          \
      }                                                                      \
    }                                                                        \
  } while (0)

void TestDocTypeSchemaValid() {
  std::cout << "DocTypeSchema: valid input\n";
  const std::string yaml = R"(
name: prd
display_name: PRD
required_sections:
  - { heading: Goal, min_words: 20 }
  - { heading: Acceptance Criteria, kind: list, min_items: 1 }
optional_sections:
  - { heading: Open Questions }
front_matter_required: [author, owner_agent]
)";
  auto r = DocTypeSchema::LoadFromString(yaml, "test://prd.yaml");
  EXPECT_OK(r);
  if (r.ok()) {
    assert(r.value().name() == "prd");
    assert(r.value().display_name() == "PRD");
    assert(r.value().required_sections().size() == 2);
    assert(r.value().required_sections()[1].kind == "list");
    assert(r.value().front_matter_required().size() == 2);
  }
}

void TestDocTypeSchemaMissingName() {
  std::cout << "DocTypeSchema: missing 'name'\n";
  const std::string yaml = "display_name: foo\n";
  EXPECT_ERR_CONTAINS(DocTypeSchema::LoadFromString(yaml, "t.yaml"),
                      "missing required field 'name'");
}

void TestDocTypeSchemaBadKind() {
  std::cout << "DocTypeSchema: invalid section kind\n";
  const std::string yaml = R"(
name: x
required_sections:
  - { heading: A, kind: paragraph }
)";
  EXPECT_ERR_CONTAINS(DocTypeSchema::LoadFromString(yaml, "t.yaml"),
                      "kind' must be empty or 'list'");
}

void TestDocTypeSchemaDuplicateHeading() {
  std::cout << "DocTypeSchema: duplicate heading\n";
  const std::string yaml = R"(
name: x
required_sections:
  - { heading: A }
optional_sections:
  - { heading: A }
)";
  EXPECT_ERR_CONTAINS(DocTypeSchema::LoadFromString(yaml, "t.yaml"),
                      "duplicate section heading");
}

void TestDocTypeSchemaYamlSyntaxLine() {
  std::cout << "DocTypeSchema: YAML syntax error reports a line number\n";
  const std::string yaml = "name: x\nrequired_sections: [\n";
  auto r = DocTypeSchema::LoadFromString(yaml, "t.yaml");
  if (r.ok()) {
    std::fprintf(stderr, "  FAIL: expected YAML parse error\n");
    ++failures;
  } else {
    if (r.error().line == 0) {
      std::fprintf(stderr,
                   "  FAIL: yaml-cpp didn't report a line number for parse "
                   "error: %s\n",
                   r.error().message.c_str());
      ++failures;
    }
  }
}

void TestAgentDefinitionValid() {
  std::cout << "AgentDefinition: valid input\n";
  const std::string yaml = R"(
name: product
llm: gpt-4o
system_prompt: You are the product manager.
)";
  auto r = AgentDefinition::LoadFromString(yaml, "t.yaml");
  EXPECT_OK(r);
  if (r.ok()) {
    assert(r.value().name() == "product");
    assert(r.value().kind() == "worker");
    assert(r.value().memory_namespace() == "product");
  }
}

void TestAgentDefinitionMissingFields() {
  std::cout << "AgentDefinition: each missing required field reports\n";
  EXPECT_ERR_CONTAINS(
      AgentDefinition::LoadFromString("llm: x\nsystem_prompt: y\n", "t.yaml"),
      "missing required field 'name'");
  EXPECT_ERR_CONTAINS(
      AgentDefinition::LoadFromString("name: x\nsystem_prompt: y\n", "t.yaml"),
      "missing required field 'llm'");
  EXPECT_ERR_CONTAINS(
      AgentDefinition::LoadFromString("name: x\nllm: y\n", "t.yaml"),
      "missing required field 'system_prompt'");
}

void TestAgentDefinitionBadKind() {
  std::cout << "AgentDefinition: invalid kind\n";
  const std::string yaml = R"(
name: a
kind: superworker
llm: x
system_prompt: y
)";
  EXPECT_ERR_CONTAINS(AgentDefinition::LoadFromString(yaml, "t.yaml"),
                      "'kind' must be 'worker' or 'reviewer'");
}

void TestFlowDefinitionValid() {
  std::cout << "FlowDefinition: valid input + cross-validation\n";
  const std::string yaml = R"(
name: simple-prd-to-spec
agents:
  - product
  - architect
edges:
  - { from: product, to: architect, port: prd, requires_human_approval: true }
max_review_rounds: 2
on_review_exhausted: approve
)";
  auto r = FlowDefinition::LoadFromString(yaml, "t.yaml");
  EXPECT_OK(r);
  if (r.ok()) {
    const auto& f = r.value();
    assert(f.agents().size() == 2);
    assert(f.edges().size() == 1);
    assert(f.edges()[0].requires_human_approval == true);
    assert(f.max_review_rounds() == 2);
    auto issues = f.ValidateAgainst({"product", "architect"}, {"prd"});
    assert(issues.empty());

    auto bad = f.ValidateAgainst({"product"}, {});
    if (bad.empty()) {
      std::fprintf(stderr, "  FAIL: expected cross-validation issues\n");
      ++failures;
    }
  }
}

void TestFlowDefinitionMissingAgents() {
  std::cout << "FlowDefinition: 'agents' required\n";
  EXPECT_ERR_CONTAINS(FlowDefinition::LoadFromString("name: x\n", "t.yaml"),
                      "missing required field 'agents'");
}

void TestFlowDefinitionDuplicateAgent() {
  std::cout << "FlowDefinition: duplicate agent in list\n";
  const std::string yaml = "name: x\nagents: [a, a]\n";
  EXPECT_ERR_CONTAINS(FlowDefinition::LoadFromString(yaml, "t.yaml"),
                      "duplicate agent");
}

void TestFlowDefinitionBadOnExhausted() {
  std::cout << "FlowDefinition: bad on_review_exhausted\n";
  const std::string yaml = R"(
name: x
agents: [a]
on_review_exhausted: maybe
)";
  EXPECT_ERR_CONTAINS(FlowDefinition::LoadFromString(yaml, "t.yaml"),
                      "'approve' or 'halt'");
}

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// DocumentStore tests (revisions, locking, conflict diversion).
// ---------------------------------------------------------------------------

#include <fcntl.h>
#include <sys/file.h>
#include <unistd.h>

std::filesystem::path MakeTempFlowDir(const std::string& tag) {
  auto base = std::filesystem::temp_directory_path() /
              ("cronymax_test_" + tag + "_" +
               std::to_string(::getpid()) + "_" +
               std::to_string(std::chrono::steady_clock::now()
                                  .time_since_epoch()
                                  .count()));
  // Mirror the real layout: <workspace>/.cronymax/flows/<flow-id>.
  auto flow_dir = base / ".cronymax" / "flows" / "test-flow";
  std::filesystem::create_directories(flow_dir);
  return flow_dir;
}

// The flow_dir is <tmp>/<base>/.cronymax/flows/test-flow. We delete the
// outermost <tmp>/<base> to clean up everything we created.
void CleanupFlowDir(const std::filesystem::path& flow_dir) {
  std::error_code ec;
  std::filesystem::remove_all(
      flow_dir.parent_path().parent_path().parent_path(), ec);
}

void TestDocumentStoreSubmitAndRead() {
  std::cout << "DocumentStore: submit creates current + history\n";
  auto dir = MakeTempFlowDir("submit");
  DocumentStore store(dir);
  std::string err;

  auto r1 = store.Submit("prd", "# PRD v1\nbody\n",
                         std::chrono::milliseconds(0), &err);
  if (r1.revision != 1) {
    std::fprintf(stderr, "  FAIL: expected rev 1, got %d (%s)\n",
                 r1.revision, err.c_str());
    ++failures;
    CleanupFlowDir(dir);
    return;
  }
  if (r1.sha256_hex.size() != 64) {
    std::fprintf(stderr, "  FAIL: sha256 length %zu\n",
                 r1.sha256_hex.size());
    ++failures;
  }
  assert(std::filesystem::exists(r1.doc_path));
  assert(std::filesystem::exists(r1.history_path));

  auto r2 = store.Submit("prd", "# PRD v2\n",
                         std::chrono::milliseconds(0), &err);
  if (r2.revision != 2) {
    std::fprintf(stderr, "  FAIL: expected rev 2, got %d\n", r2.revision);
    ++failures;
  }
  if (r1.sha256_hex == r2.sha256_hex) {
    std::fprintf(stderr, "  FAIL: identical sha256 across revisions\n");
    ++failures;
  }

  auto cur = store.Read("prd", &err);
  if (!cur || cur->find("v2") == std::string::npos) {
    std::fprintf(stderr, "  FAIL: Read() missing v2 content\n");
    ++failures;
  }
  auto hist1 = store.ReadRevision("prd", 1, &err);
  if (!hist1 || hist1->find("v1") == std::string::npos) {
    std::fprintf(stderr, "  FAIL: history rev 1 missing v1 content\n");
    ++failures;
  }
  if (store.LatestRevision("prd") != 2) {
    std::fprintf(stderr, "  FAIL: LatestRevision != 2\n");
    ++failures;
  }
  CleanupFlowDir(dir);
}

void TestDocumentStoreList() {
  std::cout << "DocumentStore: List returns expected docs\n";
  auto dir = MakeTempFlowDir("list");
  DocumentStore store(dir);
  std::string err;
  store.Submit("prd", "a", std::chrono::milliseconds(0), &err);
  store.Submit("spec", "b", std::chrono::milliseconds(0), &err);
  auto items = store.List();
  if (items.size() != 2) {
    std::fprintf(stderr, "  FAIL: expected 2 docs, got %zu\n", items.size());
    ++failures;
  }
  CleanupFlowDir(dir);
}

void TestDocumentStoreInvalidName() {
  std::cout << "DocumentStore: rejects unsafe document name\n";
  auto dir = MakeTempFlowDir("invalid");
  DocumentStore store(dir);
  std::string err;
  auto r = store.Submit("../escape", "x",
                        std::chrono::milliseconds(0), &err);
  if (r.revision != 0) {
    std::fprintf(stderr, "  FAIL: expected reject, got rev %d\n", r.revision);
    ++failures;
  }
  if (err.find("invalid") == std::string::npos) {
    std::fprintf(stderr, "  FAIL: missing 'invalid' in error: %s\n",
                 err.c_str());
    ++failures;
  }
  CleanupFlowDir(dir);
}

void TestDocumentStoreLockContention() {
  std::cout << "DocumentStore: zero-timeout fails under contention\n";
  auto dir = MakeTempFlowDir("lock");
  DocumentStore store(dir);
  std::string err;
  store.Submit("prd", "seed", std::chrono::milliseconds(0), &err);

  // Pre-acquire the doc's lock from another fd to simulate a concurrent
  // writer; this is exactly what a second DocumentStore instance would
  // do internally.
  auto lock_path = store.LocksDir() / "prd.lock";
  int fd = ::open(lock_path.c_str(), O_RDWR | O_CREAT, 0644);
  assert(fd >= 0);
  if (::flock(fd, LOCK_EX | LOCK_NB) != 0) {
    std::fprintf(stderr, "  FAIL: couldn't pre-acquire test lock\n");
    ++failures;
    ::close(fd);
    CleanupFlowDir(dir);
    return;
  }

  err.clear();
  auto r = store.Submit("prd", "contended",
                        std::chrono::milliseconds(0), &err);
  if (r.revision != 0) {
    std::fprintf(stderr, "  FAIL: expected lock failure, got rev %d\n",
                 r.revision);
    ++failures;
  }
  if (err.find("lock") == std::string::npos) {
    std::fprintf(stderr, "  FAIL: missing 'lock' in error: %s\n",
                 err.c_str());
    ++failures;
  }

  ::flock(fd, LOCK_UN);
  ::close(fd);
  CleanupFlowDir(dir);
}

void TestDocumentStoreConflictDiversion() {
  std::cout << "DocumentStore: DivertToConflict writes under conflicts/\n";
  auto dir = MakeTempFlowDir("conflict");
  DocumentStore store(dir);
  std::string err;
  auto p = store.DivertToConflict("prd", "external edit\n", &err);
  if (p.empty() || !std::filesystem::exists(p)) {
    std::fprintf(stderr, "  FAIL: conflict path missing: %s (%s)\n",
                 p.c_str(), err.c_str());
    ++failures;
  }
  if (p.parent_path().filename() != "conflicts") {
    std::fprintf(stderr, "  FAIL: wrong parent: %s\n",
                 p.parent_path().c_str());
    ++failures;
  }
  CleanupFlowDir(dir);
}

// ---------- Reviews subsystem (group 6) ----------

void TestReviewsStateRoundTrip() {
  std::cout << "ReviewsState: JSON round-trip preserves comments+revisions\n";
  ReviewsState s;
  auto& d = s.docs["prd"];
  d.current_revision = 2;
  d.status = DocStatus::kInReview;
  d.round_count = 1;
  d.revisions.push_back({1, "2025-01-01T00:00:00Z", "product",
                         "deadbeefdeadbeefdeadbeefdeadbeef"});
  d.revisions.push_back({2, "2025-01-02T00:00:00Z", "product",
                         "cafef00dcafef00dcafef00dcafef00d"});
  DocComment c;
  c.id = "c-1"; c.author = "schema"; c.kind = "changes_requested";
  c.anchor = "rev=1"; c.body = "missing acceptance criteria";
  d.comments.push_back(c);

  auto json = s.ToJson();
  ReviewsState parsed;
  std::string err;
  if (!ReviewsState::FromJson(json, &parsed, &err)) {
    std::fprintf(stderr, "  FAIL: parse: %s\n", err.c_str());
    ++failures; return;
  }
  if (parsed.docs.size() != 1 || parsed.docs.count("prd") != 1) {
    std::fprintf(stderr, "  FAIL: missing prd\n"); ++failures; return;
  }
  const auto& d2 = parsed.docs["prd"];
  if (d2.current_revision != 2 || d2.round_count != 1 ||
      d2.status != DocStatus::kInReview) {
    std::fprintf(stderr, "  FAIL: scalar fields wrong\n"); ++failures;
  }
  if (d2.revisions.size() != 2 || d2.revisions[1].sha.size() != 32) {
    std::fprintf(stderr, "  FAIL: revisions wrong\n"); ++failures;
  }
  if (d2.comments.size() != 1 || d2.comments[0].kind != "changes_requested") {
    std::fprintf(stderr, "  FAIL: comments wrong\n"); ++failures;
  }
}

void TestReviewStoreAtomicUpdate() {
  std::cout << "ReviewStore: Update appends comments under flock\n";
  auto base = MakeTempFlowDir("review");
  auto run_dir = base / "runs" / "run-A";
  std::filesystem::create_directories(run_dir);
  ReviewStore rs(run_dir);

  std::string err;
  bool ok = rs.Update([](ReviewsState& s) {
    auto& d = s.docs["spec"];
    d.current_revision = 1;
    DocComment c; c.id = "c-1"; c.author = "user"; c.kind = "comment";
    c.body = "first"; d.comments.push_back(c);
    return true;
  }, std::chrono::milliseconds(1000), &err);
  if (!ok) { std::fprintf(stderr, "  FAIL: 1st update: %s\n", err.c_str()); ++failures; }

  ok = rs.Update([](ReviewsState& s) {
    auto& d = s.docs["spec"];
    DocComment c; c.id = "c-2"; c.author = "user"; c.kind = "comment";
    c.body = "second"; d.comments.push_back(c);
    return true;
  }, std::chrono::milliseconds(1000), &err);
  if (!ok) { std::fprintf(stderr, "  FAIL: 2nd update: %s\n", err.c_str()); ++failures; }

  ReviewsState loaded;
  if (!rs.Load(&loaded, &err)) {
    std::fprintf(stderr, "  FAIL: load: %s\n", err.c_str()); ++failures;
  }
  if (loaded.docs["spec"].comments.size() != 2) {
    std::fprintf(stderr, "  FAIL: expected 2 comments, got %zu\n",
                 loaded.docs["spec"].comments.size());
    ++failures;
  }
  CleanupFlowDir(base);
}

void TestReviewStoreLockBlocks() {
  std::cout << "ReviewStore: contended lock returns 'lock' error\n";
  auto base = MakeTempFlowDir("revlock");
  auto run_dir = base / "runs" / "run-B";
  std::filesystem::create_directories(run_dir);
  ReviewStore rs(run_dir);

  // Pre-acquire the lock from an independent fd to simulate contention.
  auto lock_path = run_dir / "reviews.lock";
  int fd = ::open(lock_path.c_str(), O_RDWR | O_CREAT, 0644);
  assert(fd >= 0);
  assert(::flock(fd, LOCK_EX | LOCK_NB) == 0);

  std::string err;
  bool ok = rs.Update([](ReviewsState&) { return true; },
                      std::chrono::milliseconds(20), &err);
  if (ok) {
    std::fprintf(stderr, "  FAIL: expected lock contention\n"); ++failures;
  }
  if (err.find("lock") == std::string::npos) {
    std::fprintf(stderr, "  FAIL: expected 'lock' in error: %s\n", err.c_str());
    ++failures;
  }
  ::flock(fd, LOCK_UN); ::close(fd);
  CleanupFlowDir(base);
}

// ── Block-ID anchored review extensions ────────────────────────────────
//
// Covers `change: document-wysiwyg`. Scope is the data layer (struct
// round-trip, lazy migration, suggestion-resolved state). The wire-level
// `document.suggestion.apply` channel is exercised by the renderer Zod
// schema test in `web/test/`; bridge_handler isn't built by loader_test
// (CRONYMAX_BUILD_APP=OFF), so its block-replacement logic is asserted
// via the equivalent steps directly using DocumentStore + ReviewStore.
//
void TestReviewsState_BlockIdRoundTrip() {
  std::cout << "ReviewsState: block_id/suggestion/legacy_anchor round-trip\n";
  ReviewsState s;
  auto& d = s.docs["prd"];
  d.current_revision = 3;
  d.status = DocStatus::kInReview;
  DocComment c;
  c.id = "c-7"; c.author = "user"; c.kind = "comment";
  c.anchor = "block=11111111-2222-3333-4444-555555555555";
  c.body = "tighten the wording";
  c.block_id = "11111111-2222-3333-4444-555555555555";
  c.suggestion = "Tighter wording goes here.\n";
  c.legacy_anchor = "rev=2 lines=10-12";
  d.comments.push_back(c);

  auto json = s.ToJson();
  ReviewsState parsed;
  std::string err;
  if (!ReviewsState::FromJson(json, &parsed, &err)) {
    std::fprintf(stderr, "  FAIL: parse: %s\n", err.c_str());
    ++failures; return;
  }
  const auto& got = parsed.docs["prd"].comments[0];
  if (got.block_id != c.block_id) {
    std::fprintf(stderr, "  FAIL: block_id round-trip: got %s\n",
                 got.block_id.c_str());
    ++failures;
  }
  if (got.suggestion != c.suggestion) {
    std::fprintf(stderr, "  FAIL: suggestion round-trip\n"); ++failures;
  }
  if (got.legacy_anchor != c.legacy_anchor) {
    std::fprintf(stderr, "  FAIL: legacy_anchor round-trip\n"); ++failures;
  }

  // Stability: a comment with all three fields empty must NOT emit them
  // (keeps legacy reviews.json files byte-stable on save).
  ReviewsState s2;
  auto& d2 = s2.docs["prd"];
  d2.current_revision = 1;
  DocComment c2;
  c2.id = "c-1"; c2.author = "schema"; c2.kind = "changes_requested";
  c2.anchor = "rev=1"; c2.body = "missing section";
  d2.comments.push_back(c2);
  auto j2 = s2.ToJson();
  if (j2.find("block_id") != std::string::npos ||
      j2.find("suggestion") != std::string::npos ||
      j2.find("legacy_anchor") != std::string::npos) {
    std::fprintf(stderr,
                 "  FAIL: empty fields leaked into JSON: %s\n", j2.c_str());
    ++failures;
  }
}

void TestReviewStore_LazyMigration_LineRangeToBlockId() {
  std::cout << "ReviewStore::MigrateAnchors: line-range → block_id\n";
  auto base = MakeTempFlowDir("migrate");
  auto run_dir = base / "runs" / "run-M";
  std::filesystem::create_directories(run_dir);
  ReviewStore rs(run_dir);

  // Seed a legacy comment anchored as "rev=1 lines=3-3".
  std::string err;
  bool ok = rs.Update([](ReviewsState& s) {
    auto& d = s.docs["prd"];
    d.current_revision = 1;
    DocComment c;
    c.id = "c-legacy"; c.author = "user"; c.kind = "comment";
    c.anchor = "rev=1 lines=3-3"; c.body = "tighten";
    d.comments.push_back(c);
    return true;
  }, std::chrono::milliseconds(1000), &err);
  if (!ok) {
    std::fprintf(stderr, "  FAIL: seed: %s\n", err.c_str()); ++failures;
    CleanupFlowDir(base); return;
  }

  // Revision body: line 3 (1-based) is the paragraph, line 2 is the marker.
  // Index   0: "# Title"
  // Index   1: "<!-- block: aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee -->"
  // Index   2: "Body paragraph."
  const std::string rev_body =
      "# Title\n"
      "<!-- block: aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee -->\n"
      "Body paragraph.\n";

  auto loader = [&](const std::string& doc_name,
                    int rev) -> std::optional<std::string> {
    if (doc_name == "prd" && rev == 1) return rev_body;
    return std::nullopt;
  };
  ok = rs.MigrateAnchors(loader, std::chrono::milliseconds(1000), &err);
  if (!ok) {
    std::fprintf(stderr, "  FAIL: migrate: %s\n", err.c_str()); ++failures;
    CleanupFlowDir(base); return;
  }

  ReviewsState after;
  if (!rs.Load(&after, &err)) {
    std::fprintf(stderr, "  FAIL: post-load: %s\n", err.c_str()); ++failures;
    CleanupFlowDir(base); return;
  }
  const auto& got = after.docs["prd"].comments[0];
  if (got.block_id != "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee") {
    std::fprintf(stderr,
                 "  FAIL: block_id not migrated: %s\n", got.block_id.c_str());
    ++failures;
  }
  if (got.legacy_anchor != "rev=1 lines=3-3") {
    std::fprintf(stderr, "  FAIL: legacy_anchor not preserved: %s\n",
                 got.legacy_anchor.c_str());
    ++failures;
  }
  if (got.anchor.find("block=") != 0) {
    std::fprintf(stderr, "  FAIL: anchor not rewritten: %s\n",
                 got.anchor.c_str());
    ++failures;
  }

  // Idempotency: a second migration pass must be a no-op (no error).
  ok = rs.MigrateAnchors(loader, std::chrono::milliseconds(1000), &err);
  if (!ok) {
    std::fprintf(stderr, "  FAIL: idempotent migrate: %s\n", err.c_str());
    ++failures;
  }
  CleanupFlowDir(base);
}

void TestSuggestionApply_DataLayer() {
  std::cout << "SuggestionApply: doc submit + comment.resolved_in_rev\n";
  // Bridge dispatcher isn't linked into loader_test (the renderer-facing
  // channel is asserted by the Zod schema test). This test exercises the
  // two side effects the bridge handler is responsible for: (1) submit a
  // new revision via DocumentStore, and (2) mark `resolved_in_rev` on the
  // originating comment via ReviewStore::Update — the same calls the
  // handler makes after replacing the block body.
  auto base = MakeTempFlowDir("sugg");
  auto run_dir = base / "runs" / "run-S";
  std::filesystem::create_directories(run_dir);

  DocumentStore docs(base);
  std::string err;
  auto wr1 = docs.Submit(
      "prd",
      "# Title\n"
      "<!-- block: bbbbbbbb-cccc-dddd-eeee-ffffffffffff -->\n"
      "Original body.\n",
      std::chrono::milliseconds(1000), &err);
  if (wr1.revision != 1) {
    std::fprintf(stderr, "  FAIL: submit r1: %s\n", err.c_str()); ++failures;
    CleanupFlowDir(base); return;
  }

  ReviewStore reviews(run_dir);
  bool ok = reviews.Update([](ReviewsState& s) {
    auto& d = s.docs["prd"];
    d.current_revision = 1;
    DocComment c;
    c.id = "c-sugg"; c.author = "user"; c.kind = "comment";
    c.anchor = "block=bbbbbbbb-cccc-dddd-eeee-ffffffffffff";
    c.body = "tighter please";
    c.block_id = "bbbbbbbb-cccc-dddd-eeee-ffffffffffff";
    c.suggestion = "Tightened body.";
    d.comments.push_back(c);
    return true;
  }, std::chrono::milliseconds(1000), &err);
  if (!ok) {
    std::fprintf(stderr, "  FAIL: seed: %s\n", err.c_str()); ++failures;
    CleanupFlowDir(base); return;
  }

  // Apply: write a new revision with the block replaced.
  auto wr2 = docs.Submit(
      "prd",
      "# Title\n"
      "<!-- block: bbbbbbbb-cccc-dddd-eeee-ffffffffffff -->\n"
      "Tightened body.\n",
      std::chrono::milliseconds(1000), &err);
  if (wr2.revision != 2) {
    std::fprintf(stderr, "  FAIL: submit r2: %s\n", err.c_str()); ++failures;
    CleanupFlowDir(base); return;
  }

  // Mark the comment resolved at the new revision.
  ok = reviews.Update([&](ReviewsState& s) {
    auto& d = s.docs["prd"];
    for (auto& c : d.comments) {
      if (c.id == "c-sugg") c.resolved_in_rev = wr2.revision;
    }
    d.current_revision = wr2.revision;
    return true;
  }, std::chrono::milliseconds(1000), &err);
  if (!ok) {
    std::fprintf(stderr, "  FAIL: resolve update: %s\n", err.c_str());
    ++failures;
  }

  ReviewsState final_state;
  if (!reviews.Load(&final_state, &err)) {
    std::fprintf(stderr, "  FAIL: final load: %s\n", err.c_str()); ++failures;
    CleanupFlowDir(base); return;
  }
  const auto& cf = final_state.docs["prd"].comments[0];
  if (!cf.resolved_in_rev.has_value() || *cf.resolved_in_rev != 2) {
    std::fprintf(stderr, "  FAIL: resolved_in_rev=%d (expected 2)\n",
                 cf.resolved_in_rev.value_or(-1));
    ++failures;
  }
  if (final_state.docs["prd"].current_revision != 2) {
    std::fprintf(stderr, "  FAIL: current_revision not bumped\n"); ++failures;
  }

  // Negative cases mirror the bridge handler's 400/409 returns:
  //   - missing block_id  → bridge returns 400 (unit-tested by the Zod
  //     req schema in web/test/); here we just assert that a comment
  //     with empty block_id is correctly distinguishable.
  //   - missing suggestion → same pattern.
  //   - stale revision     → bridge inspects legacy_anchor and bumps to
  //     409 if `rev=N` < current; covered by the migration test above.
  DocComment empty;
  if (!empty.block_id.empty() || !empty.suggestion.empty()) {
    std::fprintf(stderr, "  FAIL: default DocComment fields not empty\n");
    ++failures;
  }
  CleanupFlowDir(base);
}

void TestSchemaReviewerMissingSection() {
  std::cout << "SchemaReviewer: flags missing required section\n";
  const std::string yaml =
    "name: prd\n"
    "display_name: Product Requirements\n"
    "required_sections:\n"
    "  - heading: Overview\n"
    "  - heading: Acceptance Criteria\n"
    "    kind: list\n"
    "    min_items: 1\n";
  auto sr = DocTypeSchema::LoadFromString(yaml, "test://prd.yaml");
  assert(sr.ok());
  const auto& schema = sr.value();

  // Missing "Acceptance Criteria"
  auto v = SchemaReviewer::Review(
      schema, "## Overview\n\nSome text.\n");
  if (v.ok) { std::fprintf(stderr, "  FAIL: expected !ok\n"); ++failures; }
  bool found = false;
  for (const auto& f : v.findings) {
    if (f.body.find("Acceptance Criteria") != std::string::npos) found = true;
  }
  if (!found) {
    std::fprintf(stderr, "  FAIL: missing-section finding not present\n");
    ++failures;
  }
}

void TestSchemaReviewerPasses() {
  std::cout << "SchemaReviewer: passes complete document\n";
  const std::string yaml =
    "name: prd\n"
    "display_name: Product Requirements\n"
    "required_sections:\n"
    "  - heading: Overview\n";
  auto sr = DocTypeSchema::LoadFromString(yaml, "test://prd.yaml");
  assert(sr.ok());
  auto v = SchemaReviewer::Review(sr.value(),
      "## Overview\n\nA short overview here.\n");
  if (!v.ok) {
    std::fprintf(stderr, "  FAIL: expected ok\n"); ++failures;
  }
}

void TestReviewerPipelineExhaustion() {
  std::cout << "ReviewerPipeline: max_review_rounds + on_exhausted=halt\n";
  const std::string yaml =
    "name: prd\n"
    "display_name: PRD\n"
    "required_sections:\n"
    "  - heading: Acceptance Criteria\n";
  auto sr = DocTypeSchema::LoadFromString(yaml, "test://prd.yaml");
  assert(sr.ok());
  ReviewerPolicy policy;
  policy.max_review_rounds = 2;
  policy.on_exhausted = ReviewerPolicy::OnExhausted::kHalt;
  ReviewerPipeline pipe(sr.value(), policy, {});

  // Document missing Acceptance Criteria → schema reviewer always fails.
  auto out = pipe.Run("## Overview\nbody\n", /*current_round=*/2);
  if (out.status != PipelineOutcome::Status::kHalt) {
    std::fprintf(stderr, "  FAIL: expected kHalt, got %d\n",
                 static_cast<int>(out.status)); ++failures;
  }
}

void TestReviewerPipelineApprove() {
  std::cout << "ReviewerPipeline: passes when schema satisfied\n";
  const std::string yaml =
    "name: prd\n"
    "display_name: PRD\n"
    "required_sections:\n"
    "  - heading: Overview\n";
  auto sr = DocTypeSchema::LoadFromString(yaml, "test://prd.yaml");
  assert(sr.ok());
  ReviewerPipeline pipe(sr.value(), ReviewerPolicy{}, {});
  auto out = pipe.Run("## Overview\nplenty of words here.\n", 1);
  if (out.status != PipelineOutcome::Status::kApproved) {
    std::fprintf(stderr, "  FAIL: expected kApproved\n"); ++failures;
  }
}

void TestReviewerPipelineTimeoutDoesNotBlock() {
  std::cout << "ReviewerPipeline: LLM reviewer timeout doesn't stall\n";
  const std::string yaml =
    "name: prd\n"
    "display_name: PRD\n"
    "required_sections:\n"
    "  - heading: Overview\n";
  auto sr = DocTypeSchema::LoadFromString(yaml, "test://prd.yaml");
  assert(sr.ok());
  ReviewerPolicy policy;
  policy.reviewer_timeout = std::chrono::seconds(0);  // immediately expired
  std::vector<LlmReviewer> reviewers = {
    LlmReviewer{"slow", [](const std::string&, std::chrono::seconds) {
      // Block past the deadline; pipeline should report timeout, not hang.
      std::this_thread::sleep_for(std::chrono::seconds(5));
      return ReviewerVerdict{};
    }}
  };
  ReviewerPipeline pipe(sr.value(), policy, std::move(reviewers));
  auto start = std::chrono::steady_clock::now();
  auto out = pipe.Run("## Overview\nok.\n", 1);
  auto elapsed = std::chrono::steady_clock::now() - start;
  if (elapsed > std::chrono::seconds(4)) {
    std::fprintf(stderr, "  FAIL: pipeline waited too long\n"); ++failures;
  }
  // Timed-out reviewer is non-blocking → schema passes → kApproved.
  if (out.status != PipelineOutcome::Status::kApproved) {
    std::fprintf(stderr, "  FAIL: expected kApproved on timeout, got %d\n",
                 static_cast<int>(out.status)); ++failures;
  }
}

// ---------- AgentRuntime evolution (group 7) ----------

void TestAgentRuntimeProtectedFileWrite() {
  std::cout << "AgentRuntime: file.write rejects .cronymax/ protected paths\n";
  auto base = MakeTempFlowDir("protect");
  // workspace_root for AgentRuntime is the workspace, not the flow dir.
  auto workspace = base.parent_path().parent_path().parent_path();
  AgentRuntime rt(workspace, AgentIdentity{"a", "r1", "a"}, FlowBindings{});

  ToolCall call;
  call.name = "file.write";
  call.input = ".cronymax/agents/evil.agent.yaml\nbody";
  auto r = rt.tools().Invoke(call);
  if (r.ok) { std::fprintf(stderr, "  FAIL: expected protected reject\n"); ++failures; }
  if (r.error.find("protected") == std::string::npos) {
    std::fprintf(stderr, "  FAIL: missing 'protected' in error: %s\n",
                 r.error.c_str());
    ++failures;
  }

  // Plain workspace files still work.
  ToolCall ok_call{"file.write", "scratch.txt\nhello"};
  auto r2 = rt.tools().Invoke(ok_call);
  if (!r2.ok) {
    std::fprintf(stderr, "  FAIL: ordinary write rejected: %s\n",
                 r2.error.c_str());
    ++failures;
  }
  CleanupFlowDir(base);
}

void TestAgentRuntimeSubmitDocument() {
  std::cout << "AgentRuntime: submit_document writes via DocumentStore + reviews\n";
  auto base = MakeTempFlowDir("submit");
  auto workspace = base.parent_path().parent_path().parent_path();
  auto run_dir = base / "runs" / "r-1";
  std::filesystem::create_directories(run_dir);

  auto store = std::make_shared<cronymax::DocumentStore>(base);
  auto reviews = std::make_shared<cronymax::ReviewStore>(run_dir);
  FlowBindings fb;
  fb.flow_id = "demo-flow";
  fb.document_store = store;
  fb.review_store = reviews;
  fb.producing_type = "prd";

  AgentRuntime rt(workspace, AgentIdentity{"product", "r-1", "product"},
                  std::move(fb));
  // Two AgentRuntime instances coexist on the same workspace; confirm
  // tool registries are independent (no shared state).
  AgentRuntime rt2(workspace, AgentIdentity{"critic", "r-1", "critic"},
                   FlowBindings{});
  if (rt2.tools().Invoke({"submit_document", "x\nbody"}).ok) {
    std::fprintf(stderr, "  FAIL: rt2 should not have working submit\n");
    ++failures;
  }

  ToolCall call{"submit_document", "prd-001\ntype:prd\n# PRD body\n"};
  auto r = rt.tools().Invoke(call);
  if (!r.ok) {
    std::fprintf(stderr, "  FAIL: submit returned error: %s\n", r.error.c_str());
    ++failures;
  }
  if (!rt.last_tool_was_terminal()) {
    std::fprintf(stderr, "  FAIL: terminal flag not set\n"); ++failures;
  }
  if (store->LatestRevision("prd-001") != 1) {
    std::fprintf(stderr, "  FAIL: expected rev 1\n"); ++failures;
  }

  ReviewsState s; std::string err;
  if (!reviews->Load(&s, &err)) {
    std::fprintf(stderr, "  FAIL: load reviews: %s\n", err.c_str()); ++failures;
  }
  if (s.docs["prd-001"].current_revision != 1 ||
      s.docs["prd-001"].status != DocStatus::kInReview ||
      s.docs["prd-001"].revisions.size() != 1) {
    std::fprintf(stderr, "  FAIL: reviews state not updated\n"); ++failures;
  }

  // Wrong type rejected.
  ToolCall bad{"submit_document", "spec-001\ntype:tech-spec\nbody\n"};
  auto rb = rt.tools().Invoke(bad);
  if (rb.ok || rb.error.find("producing port") == std::string::npos) {
    std::fprintf(stderr, "  FAIL: type mismatch not rejected: %s\n",
                 rb.error.c_str()); ++failures;
  }
  CleanupFlowDir(base);
}

void TestReviewerPipelineTimeoutDoesNotBlock_Sentinel() {}  // placeholder

void TestFlowRuntimeStartCancelAndRehydrate() {
  std::cout
      << "FlowRuntime: StartRun persists state.json + cancel + rehydrate\n";
  auto base = MakeTempFlowDir("flowrt");
  auto workspace = base.parent_path().parent_path().parent_path();
  // FlowRegistry needs the flows dir to exist with a flow.yaml.
  auto flow_yaml = base / "flow.yaml";
  {
    std::ofstream out(flow_yaml);
    out << "name: demo\n"
           "agents: [product, critic]\n"
           "edges:\n"
           "  - {from: product, to: critic, port: prd}\n";
  }
  FlowRegistry registry(base.parent_path());  // .cronymax/flows
  registry.Refresh();
  if (!registry.Get("test-flow")) {
    std::fprintf(stderr, "  FAIL: flow not loaded\n"); ++failures;
    CleanupFlowDir(base); return;
  }

  std::vector<std::string> events;
  {
    FlowRuntime rt(workspace, &registry, nullptr, nullptr);
    rt.SetEventEmitter([&](const std::string& evt, const std::string&) {
      events.push_back(evt);
    });

    std::string err;
    auto run_id = rt.StartRun("test-flow", "build a thing", &err);
    if (run_id.empty()) {
      std::fprintf(stderr, "  FAIL: StartRun: %s\n", err.c_str());
      ++failures; CleanupFlowDir(base); return;
    }
    auto state = rt.GetRun(run_id);
    if (!state || state->status != FlowRunStatus::kRunning) {
      std::fprintf(stderr, "  FAIL: state not RUNNING\n"); ++failures;
    }
    auto agent = rt.GetAgent(run_id, "product");
    if (!agent) {
      std::fprintf(stderr, "  FAIL: entry agent not registered\n"); ++failures;
    }
    // state.json must exist on disk.
    auto state_path = base / "runs" / run_id / "state.json";
    if (!std::filesystem::exists(state_path)) {
      std::fprintf(stderr, "  FAIL: state.json not persisted\n"); ++failures;
    }

    if (!rt.CancelRun(run_id, &err)) {
      std::fprintf(stderr, "  FAIL: cancel: %s\n", err.c_str()); ++failures;
    }
    state = rt.GetRun(run_id);
    if (!state || state->status != FlowRunStatus::kCancelled) {
      std::fprintf(stderr, "  FAIL: state not CANCELLED\n"); ++failures;
    }
  }

  // Fresh FlowRuntime: rehydrate from disk should restore the cancelled
  // run and any RUNNING run as PAUSED. We only have a CANCELLED one.
  FlowRuntime rt2(workspace, &registry, nullptr, nullptr);
  rt2.RehydrateFromDisk();
  auto runs = rt2.ListRuns();
  if (runs.size() != 1 || runs.front()->status != FlowRunStatus::kCancelled) {
    std::fprintf(stderr, "  FAIL: rehydrate did not restore run\n"); ++failures;
  }

  if (events.empty()) {
    std::fprintf(stderr, "  FAIL: no events emitted\n"); ++failures;
  }
  CleanupFlowDir(base);
}

// Group 4 task 4.7: with EventBus wired, FlowRuntime emits AppEvents
// (kind=system, payload.subkind=run_started) instead of legacy TraceEvents.
void TestFlowRuntimeWithEventBus() {
  std::cout
      << "FlowRuntime+EventBus: StartRun appends kind=system run_started\n";
  auto base = MakeTempFlowDir("flowrt-bus");
  auto workspace = base.parent_path().parent_path().parent_path();
  auto flow_yaml = base / "flow.yaml";
  {
    std::ofstream out(flow_yaml);
    out << "name: demo\nagents: [product]\nedges: []\n";
  }
  FlowRegistry registry(base.parent_path());
  registry.Refresh();
  if (!registry.Get("test-flow")) {
    std::fprintf(stderr, "  FAIL: flow not loaded\n"); ++failures;
    CleanupFlowDir(base); return;
  }

  auto db_dir = workspace / ".cronymax";
  std::filesystem::create_directories(db_dir);
  cronymax::SpaceStore store;
  if (!store.Open(db_dir / "space.db")) {
    std::fprintf(stderr, "  FAIL: SpaceStore::Open\n"); ++failures;
    CleanupFlowDir(base); return;
  }
  cronymax::event_bus::EventBus bus(&store, "space-test", workspace);

  FlowRuntime rt(workspace, &registry, nullptr, nullptr);
  rt.SetSpaceId("space-test");
  rt.SetEventBus(&bus);
  std::string err;
  auto run_id = rt.StartRun("test-flow", "build a thing", &err);
  if (run_id.empty()) {
    std::fprintf(stderr, "  FAIL: StartRun: %s\n", err.c_str());
    ++failures; CleanupFlowDir(base); return;
  }

  // List should now show one event: kind=system, payload.subkind=run_started.
  cronymax::event_bus::ListQuery q;
  q.scope.run_id = run_id;
  auto res = bus.List(q);
  if (res.events.size() != 1) {
    std::fprintf(stderr, "  FAIL: expected 1 event, got %zu\n",
                 res.events.size()); ++failures;
  } else {
    const auto& e = res.events.front();
    if (e.kind != cronymax::event_bus::AppEventKind::kSystem) {
      std::fprintf(stderr, "  FAIL: kind not system\n"); ++failures;
    }
    if (e.run_id != run_id) {
      std::fprintf(stderr, "  FAIL: run_id mismatch\n"); ++failures;
    }
    const auto& sk = e.payload.Get("subkind");
    if (!sk.is_string() || sk.as_string() != "run_started") {
      std::fprintf(stderr, "  FAIL: subkind != run_started\n"); ++failures;
    }
    if (e.id.empty()) {
      std::fprintf(stderr, "  FAIL: id (UUIDv7) empty\n"); ++failures;
    }
  }

  // Cancel → second event "run_cancelled".
  if (!rt.CancelRun(run_id, &err)) {
    std::fprintf(stderr, "  FAIL: cancel: %s\n", err.c_str()); ++failures;
  }
  res = bus.List(q);
  if (res.events.size() != 2) {
    std::fprintf(stderr, "  FAIL: expected 2 events after cancel, got %zu\n",
                 res.events.size()); ++failures;
  }

  CleanupFlowDir(base);
}

void TestMentionParserBasics() {
  std::cout << "MentionParser: parses @\\w+, ignores fences and email-like\n";
  const std::string text =
      "Hi @alice please review.\n"
      "Cc email@example.com (not a mention).\n"
      "```\n"
      "Inside fence: @bob is ignored.\n"
      "```\n"
      "Also ping @carol-1 and @alice again.\n";
  auto ms = cronymax::MentionParser::Parse(text);
  // Expect: alice (line 1), carol-1 (line 6), alice (line 6).
  if (ms.size() != 3) {
    std::fprintf(stderr, "  FAIL: expected 3 mentions, got %zu\n", ms.size());
    ++failures; return;
  }
  if (ms[0].name != "alice" || ms[1].name != "carol-1" ||
      ms[2].name != "alice") {
    std::fprintf(stderr, "  FAIL: wrong names: %s %s %s\n",
                 ms[0].name.c_str(), ms[1].name.c_str(), ms[2].name.c_str());
    ++failures;
  }
}

void TestRouterTypedAndMention() {
  std::cout << "Router: combines typed-port + mention, dedupes, warns unknown\n";
  const std::string yaml =
      "name: demo\n"
      "agents: [product, architect, critic]\n"
      "edges:\n"
      "  - {from: product, to: architect, port: prd}\n"
      "  - {from: product, to: critic, port: prd}\n";
  auto fd = FlowDefinition::LoadFromString(yaml, "test://flow.yaml");
  if (!fd.ok()) {
    std::fprintf(stderr, "  FAIL: load: %s\n",
                 fd.error().ToString().c_str()); ++failures; return;
  }
  // typed-port: architect + critic. Mention adds critic (dedup) + unknown.
  auto d = cronymax::Router::Route(fd.value(), "product", "prd",
                                   "Body @critic @ghost\n");
  if (d.targets.size() != 2) {
    std::fprintf(stderr, "  FAIL: expected 2 targets, got %zu\n",
                 d.targets.size()); ++failures;
  } else {
    if (d.targets[0].agent != "architect" ||
        d.targets[0].reason != "typed-port") {
      std::fprintf(stderr, "  FAIL: t0=%s/%s\n",
                   d.targets[0].agent.c_str(),
                   d.targets[0].reason.c_str()); ++failures;
    }
    if (d.targets[1].agent != "critic" ||
        d.targets[1].reason != "typed+mention") {
      std::fprintf(stderr, "  FAIL: t1=%s/%s\n",
                   d.targets[1].agent.c_str(),
                   d.targets[1].reason.c_str()); ++failures;
    }
  }
  if (d.unknown_mentions.size() != 1 || d.unknown_mentions[0] != "ghost") {
    std::fprintf(stderr, "  FAIL: unknown_mentions wrong\n"); ++failures;
  }
}

void TestRouterBackwardMentionAllowed() {
  std::cout << "Router: backward @mention to upstream agent is honoured\n";
  const std::string yaml =
      "name: demo\n"
      "agents: [product, architect]\n"
      "edges:\n"
      "  - {from: product, to: architect, port: prd}\n";
  auto fd = FlowDefinition::LoadFromString(yaml, "test://flow.yaml");
  if (!fd.ok()) { ++failures; return; }
  // architect produces tech-spec but mentions product (backward).
  auto d = cronymax::Router::Route(fd.value(), "architect", "tech-spec",
                                   "@product please reconsider\n");
  if (d.targets.size() != 1 || d.targets[0].agent != "product" ||
      d.targets[0].reason != "mention") {
    std::fprintf(stderr, "  FAIL: backward routing not produced\n");
    ++failures;
  }
}

// ---------- Trace event stream (group 10) ----------

void TestTraceWriterReplayThenLive() {
  std::cout << "TraceWriter: replay-then-live preserves order\n";
  auto tmp = std::filesystem::temp_directory_path() /
             ("cronymax_trace_" + std::to_string(::getpid()) + "_" +
              std::to_string(std::chrono::steady_clock::now()
                                 .time_since_epoch().count()));
  std::filesystem::create_directories(tmp);
  auto path = tmp / "trace.jsonl";

  // Phase 1: write 2 events, flush, drop the writer (file persists).
  {
    cronymax::TraceWriter w(path);
    cronymax::TraceEvent e1; e1.kind = cronymax::TraceKind::kRunStarted;
    e1.ts_ms = 1; e1.run_id = "r1"; w.Append(e1);
    cronymax::TraceEvent e2; e2.kind = cronymax::TraceKind::kAgentStarted;
    e2.ts_ms = 2; e2.run_id = "r1"; e2.agent_id = "a1"; w.Append(e2);
    w.Flush();
  }

  // Phase 2: new writer, subscribe-replay, then write a live event.
  cronymax::TraceWriter w2(path);
  std::vector<std::string> seen;
  auto tok = w2.SubscribeReplay(
      [&](const cronymax::TraceEvent& evt) {
        seen.push_back(cronymax::TraceKindToString(evt.kind));
      });
  cronymax::TraceEvent e3; e3.kind = cronymax::TraceKind::kRunCompleted;
  e3.ts_ms = 3; e3.run_id = "r1"; w2.Append(e3);
  w2.Flush();
  w2.Unsubscribe(tok);

  if (seen.size() != 3 || seen[0] != "run.started" ||
      seen[1] != "agent.started" || seen[2] != "run.completed") {
    std::fprintf(stderr,
                 "  FAIL: ordering wrong (size=%zu)\n", seen.size());
    for (const auto& s : seen)
      std::fprintf(stderr, "    %s\n", s.c_str());
    ++failures;
  }

  std::error_code ec;
  std::filesystem::remove_all(tmp, ec);
}

// -------------------------------------------------------------------------
// Group 13 integration coverage. These tests exercise multi-component
// behaviour beyond the unit boundaries (FlowRuntime + ReviewerPipeline +
// rehydration semantics) without requiring a live LLM.
// -------------------------------------------------------------------------

void TestFlowRuntimeRehydratesPausedFromRunning() {
  std::cout << "FlowRuntime: rehydrate converts RUNNING run to PAUSED\n";
  auto base = MakeTempFlowDir("paused");
  auto workspace = base.parent_path().parent_path().parent_path();
  {
    std::ofstream out(base / "flow.yaml");
    out << "name: demo\n"
           "agents: [product, critic]\n"
           "edges:\n"
           "  - {from: product, to: critic, port: prd}\n";
  }
  FlowRegistry registry(base.parent_path());
  registry.Refresh();
  std::string run_id;
  {
    FlowRuntime rt(workspace, &registry, nullptr, nullptr);
    std::string err;
    run_id = rt.StartRun("test-flow", "go", &err);
    if (run_id.empty()) {
      std::fprintf(stderr, "  FAIL: StartRun: %s\n", err.c_str()); ++failures;
      CleanupFlowDir(base); return;
    }
    // RUNNING state is now persisted; we drop the runtime without cancel.
  }
  // Fresh runtime must rehydrate the orphaned RUNNING run as PAUSED so
  // the user can resume explicitly (Decision 7).
  FlowRuntime rt2(workspace, &registry, nullptr, nullptr);
  int paused = rt2.RehydrateFromDisk();
  if (paused != 1) {
    std::fprintf(stderr, "  FAIL: paused=%d (expected 1)\n", paused);
    ++failures;
  }
  auto state = rt2.GetRun(run_id);
  if (!state || state->status != FlowRunStatus::kPaused) {
    std::fprintf(stderr, "  FAIL: run not in PAUSED state\n"); ++failures;
  }
  CleanupFlowDir(base);
}

void TestReviewerPipelineExhaustionApproveMode() {
  std::cout
      << "ReviewerPipeline: max_review_rounds + on_exhausted=approve\n";
  const std::string yaml =
    "name: prd\n"
    "display_name: PRD\n"
    "required_sections:\n"
    "  - heading: Acceptance Criteria\n";
  auto sr = cronymax::DocTypeSchema::LoadFromString(yaml, "test://prd.yaml");
  assert(sr.ok());
  cronymax::ReviewerPolicy policy;
  policy.max_review_rounds = 2;
  policy.on_exhausted = cronymax::ReviewerPolicy::OnExhausted::kApprove;
  cronymax::ReviewerPipeline pipe(sr.value(), policy, {});
  // Document missing the required section. With kApprove the run advances
  // even though the schema reviewer never passes.
  auto out = pipe.Run("## Overview\nbody\n", /*current_round=*/2);
  if (out.status != cronymax::PipelineOutcome::Status::kApproved) {
    std::fprintf(stderr, "  FAIL: expected kApproved on exhaust, got %d\n",
                 static_cast<int>(out.status));
    ++failures;
  }
}

void TestFlowRuntimeTwoAgentSubmitAdvances() {
  std::cout
      << "FlowRuntime+DocumentStore: producer submit publishes a revision\n";
  auto base = MakeTempFlowDir("twoagent");
  auto workspace = base.parent_path().parent_path().parent_path();
  {
    std::ofstream out(base / "flow.yaml");
    out << "name: demo\n"
           "agents: [product, architect]\n"
           "edges:\n"
           "  - {from: product, to: architect, port: prd}\n";
  }
  cronymax::FlowRegistry registry(base.parent_path());
  registry.Refresh();
  cronymax::FlowRuntime rt(workspace, &registry, nullptr, nullptr);
  std::string err;
  auto run_id = rt.StartRun("test-flow", "kickoff", &err);
  if (run_id.empty()) {
    std::fprintf(stderr, "  FAIL: StartRun: %s\n", err.c_str()); ++failures;
    CleanupFlowDir(base); return;
  }
  // Producer (product) hands off a PRD via the shared DocumentStore.
  auto store = rt.GetDocumentStore(run_id);
  if (!store) {
    std::fprintf(stderr, "  FAIL: no doc store\n"); ++failures;
    CleanupFlowDir(base); return;
  }
  auto sub = store->Submit("prd-v1", "## Acceptance Criteria\n- one\n",
                           std::chrono::milliseconds(0), &err);
  if (sub.revision < 1) {
    std::fprintf(stderr, "  FAIL: Submit: %s\n", err.c_str()); ++failures;
  }
  // The architect side reads the same store and sees revision 1.
  auto read = store->Read("prd-v1", &err);
  if (!read.has_value() ||
      read->find("Acceptance Criteria") == std::string::npos) {
    std::fprintf(stderr, "  FAIL: architect cannot read PRD: %s\n",
                 err.c_str()); ++failures;
  }
  CleanupFlowDir(base);
}

}  // namespace

int main() {
  TestDocTypeSchemaValid();
  TestDocTypeSchemaMissingName();
  TestDocTypeSchemaBadKind();
  TestDocTypeSchemaDuplicateHeading();
  TestDocTypeSchemaYamlSyntaxLine();
  TestAgentDefinitionValid();
  TestAgentDefinitionMissingFields();
  TestAgentDefinitionBadKind();
  TestFlowDefinitionValid();
  TestFlowDefinitionMissingAgents();
  TestFlowDefinitionDuplicateAgent();
  TestFlowDefinitionBadOnExhausted();

  TestDocumentStoreSubmitAndRead();
  TestDocumentStoreList();
  TestDocumentStoreInvalidName();
  TestDocumentStoreLockContention();
  TestDocumentStoreConflictDiversion();

  TestReviewsStateRoundTrip();
  TestReviewStoreAtomicUpdate();
  TestReviewStoreLockBlocks();
  TestReviewsState_BlockIdRoundTrip();
  TestReviewStore_LazyMigration_LineRangeToBlockId();
  TestSuggestionApply_DataLayer();
  TestSchemaReviewerMissingSection();
  TestSchemaReviewerPasses();
  TestReviewerPipelineExhaustion();
  TestReviewerPipelineApprove();
  TestReviewerPipelineTimeoutDoesNotBlock();

  TestAgentRuntimeProtectedFileWrite();
  TestAgentRuntimeSubmitDocument();
  TestFlowRuntimeStartCancelAndRehydrate();
  TestFlowRuntimeWithEventBus();

  TestMentionParserBasics();
  TestRouterTypedAndMention();
  TestRouterBackwardMentionAllowed();

  TestTraceWriterReplayThenLive();

  TestFlowRuntimeRehydratesPausedFromRunning();
  TestReviewerPipelineExhaustionApproveMode();
  TestFlowRuntimeTwoAgentSubmitAdvances();

  if (failures > 0) {
    std::fprintf(stderr, "\n%d test failure(s)\n", failures);
    return 1;
  }
  std::cout << "\nAll loader tests passed.\n";
  return 0;
}
