#include "flow/trace_event.h"

#include <nlohmann/json.hpp>

namespace cronymax {

namespace {
// (helpers removed — nlohmann/json handles escaping)
}  // namespace

const char* TraceKindToString(TraceKind k) {
  switch (k) {
    case TraceKind::kRunStarted: return "run.started";
    case TraceKind::kRunCompleted: return "run.completed";
    case TraceKind::kRunCancelled: return "run.cancelled";
    case TraceKind::kRunFailed: return "run.failed";
    case TraceKind::kAgentStarted: return "agent.started";
    case TraceKind::kAgentEnded: return "agent.ended";
    case TraceKind::kToolCall: return "tool.call";
    case TraceKind::kToolResult: return "tool.result";
    case TraceKind::kDocumentSubmitted: return "document.submitted";
    case TraceKind::kReviewRequested: return "review.requested";
    case TraceKind::kReviewVerdict: return "review.verdict";
    case TraceKind::kReviewExhausted: return "review.exhausted";
    case TraceKind::kRouted: return "routed";
    case TraceKind::kMention: return "mention";
    case TraceKind::kError: return "error";
  }
  return "error";
}

std::string TraceEvent::ToJsonLine() const {
  nlohmann::json j = {{"kind", TraceKindToString(kind)}, {"ts_ms", ts_ms}};
  if (!space_id.empty())     j["space_id"]   = space_id;
  if (!run_id.empty())       j["run_id"]     = run_id;
  if (!agent_id.empty())     j["agent_id"]   = agent_id;
  if (!tool_name.empty())    j["tool_name"]  = tool_name;
  if (!doc_name.empty())     j["doc_name"]   = doc_name;
  if (!doc_type.empty())     j["doc_type"]   = doc_type;
  if (!payload_json.empty()) {
    auto pj = nlohmann::json::parse(payload_json, nullptr, false);
    j["payload"] = pj.is_discarded() ? nlohmann::json(payload_json) : pj;
  }
  return j.dump() + "\n";
}

}  // namespace cronymax
