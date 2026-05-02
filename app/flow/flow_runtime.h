#ifndef CRONYMAX_FLOW_FLOW_RUNTIME_H_
#define CRONYMAX_FLOW_FLOW_RUNTIME_H_

#include <chrono>
#include <filesystem>
#include <functional>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

namespace cronymax {

class AgentRegistry;
class AgentRuntime;
class DocTypeRegistry;
class DocumentStore;
class FlowDefinition;
class FlowRegistry;
class ReviewStore;
class TraceWriter;

namespace event_bus {
class EventBus;
}  // namespace event_bus

// Lifecycle states for a Flow Run. Per design Decision 7, RUNNING and
// PAUSED are restored from disk as PAUSED on restart; the user must
// explicitly resume.
enum class FlowRunStatus {
  kPending,
  kRunning,
  kPaused,
  kCompleted,
  kCancelled,
  kFailed,
};

std::string FlowRunStatusToString(FlowRunStatus s);
FlowRunStatus ParseFlowRunStatus(const std::string& s);

// Per-Run document tracking entry: which Agent produced what, current rev.
struct FlowRunDocumentEntry {
  std::string name;
  std::string type;
  std::string producer_agent;
  int current_revision = 0;
};

// In-memory + persisted state for one Flow Run.
struct FlowRunState {
  std::string run_id;
  std::string flow_id;
  FlowRunStatus status = FlowRunStatus::kPending;
  std::string started_at_iso;
  std::string ended_at_iso;
  std::vector<std::string> agents_in_flight;
  std::vector<FlowRunDocumentEntry> documents;
  std::string failure_reason;
  std::string initial_input;

  std::string ToJson() const;
  static bool FromJson(const std::string& json, FlowRunState* out,
                       std::string* err);
};

// Owns active Flow Runs for one Space. Lifetime: same as Space.
//
// Thread-safety: the public API is mutex-protected. Bridge dispatch is
// expected to call into FlowRuntime from the CEF UI thread; AgentRuntime
// instances spun up by FlowRuntime do NOT execute autonomously here —
// they are exposed as data containers so the renderer's ReAct loop can
// invoke their tools via the bridge. This keeps the C++ side free of an
// LLM-driven event loop.
class FlowRuntime {
 public:
  using EventEmitter = std::function<void(const std::string& event,
                                          const std::string& json)>;

  // workspace_root is `<root>` (NOT `<root>/.cronymax`).
  FlowRuntime(std::filesystem::path workspace_root,
              FlowRegistry* flow_registry,
              AgentRegistry* agent_registry,
              DocTypeRegistry* doc_type_registry);
  ~FlowRuntime();

  FlowRuntime(const FlowRuntime&) = delete;
  FlowRuntime& operator=(const FlowRuntime&) = delete;

  // Optional event emitter — wired by SpaceManager to broadcast
  // `flow.run.changed` to the renderer.
  void SetEventEmitter(EventEmitter cb);

  // Scan `<workspace>/.cronymax/flows/*/runs/*/state.json` and reload any
  // that were RUNNING into PAUSED. Idempotent. Returns the number of
  // PAUSED runs discovered.
  int RehydrateFromDisk();

  // Start a new Run. Returns run_id on success, empty on error.
  // `initial_input` becomes the entry Agent's user prompt.
  std::string StartRun(const std::string& flow_id,
                       const std::string& initial_input,
                       std::string* err);

  // Cancel: marks status=CANCELLED, persists state, broadcasts change.
  // No-op if run doesn't exist or is already terminal.
  bool CancelRun(const std::string& run_id, std::string* err);

  // Mark a Run as completed (success path; called when the final Agent
  // submits the terminal document and review is approved).
  bool CompleteRun(const std::string& run_id, std::string* err);

  // Lookups.
  std::shared_ptr<FlowRunState> GetRun(const std::string& run_id) const;
  std::vector<std::shared_ptr<FlowRunState>> ListRuns() const;

  // Agent runtime lookup for tool dispatch from the bridge (renderer's
  // ReAct loop calls native tools via a `(run_id, agent_id)` pair).
  // Returns nullptr if not found.
  std::shared_ptr<AgentRuntime> GetAgent(const std::string& run_id,
                                         const std::string& agent_id) const;

  // Persisted shared stores (so bridge handlers like review.* can locate
  // the right stores without re-instantiating them).
  std::shared_ptr<DocumentStore> GetDocumentStore(
      const std::string& run_id) const;
  std::shared_ptr<ReviewStore> GetReviewStore(const std::string& run_id) const;

  // Per-Run trace writer (created lazily by StartRun, rehydrated on
  // RehydrateFromDisk for already-existing runs/<id>/trace.jsonl). Used
  // by the renderer's `event` channel to subscribe with replay-then-live
  // semantics.
  std::shared_ptr<TraceWriter> GetTraceWriter(
      const std::string& run_id) const;

  // Optional Space identifier — included as `space_id` in every emitted
  // TraceEvent. Set once by SpaceManager.
  void SetSpaceId(std::string id);

  // Optional event bus — when set, run lifecycle events are appended via
  // `EventBus::Append` instead of (or in addition to) the legacy
  // TraceWriter. Set once by SpaceManager after EventBus construction.
  void SetEventBus(event_bus::EventBus* bus) { event_bus_ = bus; }

 private:
  struct Run {
    std::shared_ptr<FlowRunState> state;
    std::shared_ptr<DocumentStore> doc_store;
    std::shared_ptr<ReviewStore> review_store;
    std::shared_ptr<TraceWriter> trace_writer;
    std::unordered_map<std::string, std::shared_ptr<AgentRuntime>> agents;
    std::filesystem::path run_dir;
  };

  std::filesystem::path RunDir(const std::string& flow_id,
                               const std::string& run_id) const;
  bool PersistState(const Run& run, std::string* err) const;
  void Emit(const std::string& event, const std::string& json) const;
  std::string GenerateRunId() const;

  std::filesystem::path workspace_root_;
  FlowRegistry* flow_registry_;
  AgentRegistry* agent_registry_;
  DocTypeRegistry* doc_type_registry_;
  EventEmitter emitter_;
  std::string space_id_;
  event_bus::EventBus* event_bus_ = nullptr;  // borrowed; set once.

  mutable std::mutex mu_;
  std::unordered_map<std::string, Run> runs_;  // by run_id
};

}  // namespace cronymax

#endif  // CRONYMAX_FLOW_FLOW_RUNTIME_H_
