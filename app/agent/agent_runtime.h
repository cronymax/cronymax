#pragma once

#include <chrono>
#include <filesystem>
#include <functional>
#include <memory>
#include <string>
#include <vector>

#include "agent/model_router.h"
#include "agent/tool_registry.h"
#include "workspace/file_broker.h"
#include "sandbox/sandbox_launcher.h"

namespace cronymax {

class DocumentStore;
class ReviewStore;

struct TraceEntry {
  std::string type;
  std::string message;
};

struct AgentRunResult {
  bool ok = false;
  std::string final_message;
  std::vector<TraceEntry> trace;
};

// Identity passed to every AgentRuntime instance. The defaults preserve
// the legacy single-Space prototype behavior (anonymous ad-hoc agent).
struct AgentIdentity {
  std::string agent_id;          // "" for ad-hoc / prototype runs
  std::string flow_run_id;       // "" if not running inside a Flow
  std::string memory_namespace;  // defaults to agent_id when empty
};

// Optional Flow-level wiring used by the `submit_document` tool. When
// `document_store` is non-null the tool writes the new revision through
// it; when `review_store` is non-null the doc is transitioned to
// IN_REVIEW and the revision is recorded in `reviews.json`.
struct FlowBindings {
  std::string flow_id;
  std::shared_ptr<DocumentStore> document_store;
  std::shared_ptr<ReviewStore> review_store;
  // Producing-port type (e.g. "prd"). The submit_document tool rejects
  // submissions whose `type` argument doesn't match this value, enforcing
  // typed-port discipline.
  std::string producing_type;
};

// Per-Agent runtime instance. Multiple AgentRuntime objects coexist per
// Space (one per active `(flow_run_id, agent_id)` pair). Each owns its
// own ToolRegistry, message history, and FileBroker so there is no
// cross-Agent state leakage.
//
// MIGRATION (rust-runtime-migration, group 9 / 10): this in-process
// AgentRuntime is being replaced by a runtime-managed run owned by
// `crates/cronymax`. New code MUST NOT extend AgentRuntime; new run
// lifecycle, tool dispatch, and ReAct semantics belong in the Rust
// runtime and are reached over GIPS via the (still-to-build) C++
// proxy. SpaceManager will lose its `agent_runtime_` field once the
// proxy lands. See `openspec/changes/rust-runtime-migration/`.
class AgentRuntime {
 public:
  // Legacy ctor — equivalent to AgentRuntime(workspace_root, {}, {}).
  explicit AgentRuntime(std::filesystem::path workspace_root);

  // Multi-instance ctor (added by agent-document-orchestration).
  AgentRuntime(std::filesystem::path workspace_root,
               AgentIdentity identity,
               FlowBindings flow_bindings);

  AgentRunResult RunPrototypeTask(const std::string& task,
                                  bool confirmation_granted = false);

  ToolRegistry& tools() { return tools_; }
  const ToolRegistry& tools() const { return tools_; }

  const AgentIdentity& identity() const { return identity_; }
  const FlowBindings& flow_bindings() const { return flow_bindings_; }

  // True after the most recent tool invocation was `submit_document`.
  // Renderer / FlowRuntime use this to terminate the ReAct loop.
  bool last_tool_was_terminal() const { return terminal_tool_called_; }

 private:
  void RegisterDefaultTools();
  void RegisterFlowTools();

  // Returns true if `path` resolves under a protected subtree of
  // `<workspace_root>/.cronymax/` that agents may not write directly.
  bool IsProtectedAgentWritePath(const std::filesystem::path& path) const;

  std::filesystem::path workspace_root_;
  AgentIdentity identity_;
  FlowBindings flow_bindings_;
  FileBroker file_broker_;
  SandboxLauncher sandbox_launcher_;
  ModelRouter model_router_;
  ToolRegistry tools_;
  bool terminal_tool_called_ = false;
};

}  // namespace cronymax

