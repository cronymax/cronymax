#include "agent/agent_runtime.h"

#include <chrono>
#include <ctime>
#include <sstream>
#include <system_error>

#include "agent/graph_engine.h"
#include "common/path_utils.h"
#include "document/document_store.h"
#include "document/review_store.h"
#include "document/reviews_state.h"

namespace cronymax {

namespace {
std::string IsoNowUtc() {
  auto now = std::chrono::system_clock::now();
  std::time_t t = std::chrono::system_clock::to_time_t(now);
  std::tm tm{};
  gmtime_r(&t, &tm);
  char buf[32];
  std::strftime(buf, sizeof(buf), "%Y-%m-%dT%H:%M:%SZ", &tm);
  return buf;
}
}  // namespace

AgentRuntime::AgentRuntime(std::filesystem::path workspace_root)
    : AgentRuntime(std::move(workspace_root), {}, {}) {}

AgentRuntime::AgentRuntime(std::filesystem::path workspace_root,
                           AgentIdentity identity,
                           FlowBindings flow_bindings)
    : workspace_root_(NormalizePath(workspace_root)),
      identity_(std::move(identity)),
      flow_bindings_(std::move(flow_bindings)),
      file_broker_(workspace_root_) {
  if (identity_.memory_namespace.empty()) {
    identity_.memory_namespace = identity_.agent_id;
  }
  RegisterDefaultTools();
  RegisterFlowTools();
}

bool AgentRuntime::IsProtectedAgentWritePath(
    const std::filesystem::path& path) const {
  std::error_code ec;
  auto resolved = path.is_absolute() ? path : (workspace_root_ / path);
  resolved = std::filesystem::weakly_canonical(resolved, ec);
  if (ec) resolved = path;
  auto root = std::filesystem::weakly_canonical(workspace_root_, ec);
  if (ec) root = workspace_root_;
  // Walk up checking for forbidden segments under .cronymax/.
  // Forbidden subtrees: flows/, agents/, doc-types/, and any reviews.json.
  auto rel_str = std::filesystem::relative(resolved, root, ec).generic_string();
  if (ec) return false;
  if (rel_str.rfind(".cronymax/flows/", 0) == 0 ||
      rel_str.rfind(".cronymax/agents/", 0) == 0 ||
      rel_str.rfind(".cronymax/doc-types/", 0) == 0) {
    return true;
  }
  if (rel_str.size() >= std::string("reviews.json").size() &&
      rel_str.compare(rel_str.size() - 12, 12, "reviews.json") == 0 &&
      rel_str.rfind(".cronymax/", 0) == 0) {
    return true;
  }
  return false;
}

void AgentRuntime::RegisterDefaultTools() {
  tools_.Register("file.read", [this](const ToolCall& call) {
    const auto result =
        file_broker_.ReadText(Actor::kAgent, workspace_root_ / call.input);
    if (!result.ok) {
      return ToolResult{.ok = false, .error = result.error};
    }
    return ToolResult{.ok = true, .output = result.data};
  });

  tools_.Register("file.write", [this](const ToolCall& call) {
    const auto sep = call.input.find('\n');
    if (sep == std::string::npos) {
      return ToolResult{.ok = false,
                        .error = "input must be '<path>\\n<content>'"};
    }
    const auto rel_path = call.input.substr(0, sep);
    const auto content = call.input.substr(sep + 1);
    if (IsProtectedAgentWritePath(workspace_root_ / rel_path)) {
      return ToolResult{.ok = false,
                        .error = "refusing fs.write to protected path under "
                                 ".cronymax/{flows,agents,doc-types}/ or "
                                 "reviews.json; use submit_document or "
                                 "review.* bridge channels instead"};
    }
    const auto result = file_broker_.WriteText(
        Actor::kAgent, workspace_root_ / rel_path, content);
    if (!result.ok) {
      return ToolResult{.ok = false, .error = result.error};
    }
    return ToolResult{.ok = true, .output = "wrote " + rel_path};
  });

  tools_.Register("terminal.execSandboxed", [this](const ToolCall& call) {
    const auto result = sandbox_launcher_.ExecuteShellCommand(
        Actor::kAgent, file_broker_.policy(), workspace_root_,
        call.input);
    if (result.exit_code != 0) {
      return ToolResult{.ok = false, .output = result.stdout_data,
                        .error = result.stderr_data};
    }
    return ToolResult{.ok = true, .output = result.stdout_data,
                      .error = result.stderr_data};
  });
}

AgentRunResult AgentRuntime::RunPrototypeTask(const std::string& task,
                                              bool confirmation_granted) {
  AgentRunResult run;
  run.trace.push_back({"task.received", task});
  run.trace.push_back({"model.selected", model_router_.DefaultCheapModel().id});
  GraphEngine graph_engine;
  const auto graph = graph_engine.CreatePrototypeGraph();
  const auto graph_validation = graph_engine.Validate(graph);
  run.trace.push_back({"graph.selected", graph.id});
  if (!graph_validation.ok) {
    run.ok = false;
    std::ostringstream errors;
    for (const auto& error : graph_validation.errors) {
      errors << error << "\n";
    }
    run.final_message = errors.str();
    return run;
  }

  if (task.rfind("/exec ", 0) == 0) {
    const auto command = task.substr(6);
    run.trace.push_back({"tool.request", "terminal.execSandboxed"});
    const auto result = sandbox_launcher_.ExecuteShellCommand(
        Actor::kAgent, file_broker_.policy(), workspace_root_, command,
        confirmation_granted);
    run.trace.push_back({"tool.result.exitCode", std::to_string(result.exit_code)});
    if (!result.stderr_data.empty()) {
      run.trace.push_back({"tool.result.stderr", result.stderr_data});
    }
    run.ok = result.exit_code == 0;
    run.final_message = result.stdout_data.empty() ? result.stderr_data
                                                   : result.stdout_data;
    return run;
  }

  if (task.rfind("/read ", 0) == 0) {
    const auto rel_path = task.substr(6);
    run.trace.push_back({"tool.request", "file.read"});
    const auto result =
        file_broker_.ReadText(Actor::kAgent, workspace_root_ / rel_path);
    run.ok = result.ok;
    run.final_message = result.ok ? result.data : result.error;
    return run;
  }

  if (task.rfind("/write ", 0) == 0) {
    const auto rest = task.substr(7);
    const auto sep = rest.find(' ');
    if (sep == std::string::npos) {
      run.ok = false;
      run.final_message = "usage: /write <workspace-relative-path> <content>";
      return run;
    }
    const auto rel_path = rest.substr(0, sep);
    const auto content = rest.substr(sep + 1);
    run.trace.push_back({"tool.request", "file.write"});
    const auto result = file_broker_.WriteText(
        Actor::kAgent, workspace_root_ / rel_path, content);
    run.ok = result.ok;
    run.final_message = result.ok ? "wrote " + rel_path : result.error;
    return run;
  }

  std::ostringstream help;
  help << "Prototype agent is wired. Available local commands:\n";
  help << "- /exec <shell command>\n";
  help << "- /read <workspace-relative path>\n";
  help << "- /write <workspace-relative path> <content>\n";
  help << "Registered tools:";
  for (const auto& name : tools_.ToolNames()) {
    help << "\n- " << name;
  }
  run.ok = true;
  run.final_message = help.str();
  return run;
}

// ---------------------------------------------------------------------------
// RegisterFlowTools
//
// Registers `submit_document(name\n[type\n]content)` — the *terminal* tool
// for an Agent's ReAct loop. On success the document is written through
// the bound DocumentStore (new revision + history snapshot + SHA), and
// the per-Run reviews.json is updated to record the new revision and
// flip the doc to IN_REVIEW.
//
// The tool is a no-op (returns an error) when no FlowBindings are
// attached, which preserves the legacy ad-hoc prototype runtime.
// ---------------------------------------------------------------------------

void AgentRuntime::RegisterFlowTools() {
  tools_.Register("submit_document", [this](const ToolCall& call) {
    if (!flow_bindings_.document_store) {
      return ToolResult{.ok = false,
                        .error = "submit_document is only available inside a "
                                 "Flow run; no DocumentStore bound"};
    }
    // Simple framing: first line = doc name; if a `type:` line follows,
    // it's parsed and matched against the producing port. Everything
    // after the first blank line is the body. This format is friendly
    // for hand-typed prototype calls and easy to emit from the JS loop.
    auto sep = call.input.find('\n');
    if (sep == std::string::npos) {
      return ToolResult{.ok = false,
                        .error = "input must be '<name>\\n[type:<t>\\n]<body>'"};
    }
    std::string name = call.input.substr(0, sep);
    std::string rest = call.input.substr(sep + 1);
    std::string type;
    if (rest.compare(0, 5, "type:") == 0) {
      auto eol = rest.find('\n');
      type = rest.substr(5, eol == std::string::npos ? std::string::npos
                                                     : eol - 5);
      rest = eol == std::string::npos ? std::string{} : rest.substr(eol + 1);
      // Trim whitespace around type.
      while (!type.empty() && (type.front() == ' ' || type.front() == '\t'))
        type.erase(type.begin());
      while (!type.empty() && (type.back() == ' ' || type.back() == '\t' ||
                               type.back() == '\r'))
        type.pop_back();
    }
    if (!flow_bindings_.producing_type.empty() &&
        !type.empty() && type != flow_bindings_.producing_type) {
      return ToolResult{.ok = false,
                        .error = "submit_document type '" + type +
                                 "' does not match producing port type '" +
                                 flow_bindings_.producing_type + "'"};
    }

    std::string err;
    auto wr = flow_bindings_.document_store->Submit(
        name, rest, std::chrono::milliseconds(2000), &err);
    if (wr.revision == 0) {
      return ToolResult{.ok = false, .error = err.empty() ? "submit failed" : err};
    }

    // Record revision in reviews.json (best-effort; surface failure but
    // don't fail the tool, the doc itself was written).
    if (flow_bindings_.review_store) {
      const auto submitter = identity_.agent_id.empty() ? std::string("agent")
                                                        : identity_.agent_id;
      const auto now = IsoNowUtc();
      const int rev = wr.revision;
      const auto sha = wr.sha256_hex;
      std::string rerr;
      flow_bindings_.review_store->Update(
          [&](ReviewsState& s) {
            auto& doc = s.docs[name];
            doc.current_revision = rev;
            doc.status = DocStatus::kInReview;
            doc.round_count += 1;
            doc.revisions.push_back({rev, now, submitter, sha});
            return true;
          },
          std::chrono::milliseconds(2000), &rerr);
    }

    terminal_tool_called_ = true;
    return ToolResult{.ok = true,
                      .output = "submitted " + name + " rev " +
                                std::to_string(wr.revision)};
  });
}

}  // namespace cronymax
