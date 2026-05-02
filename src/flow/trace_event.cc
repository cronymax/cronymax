#include "flow/trace_event.h"

#include <cstdio>

namespace cronymax {

namespace {

std::string EscapeJson(const std::string& in) {
  std::string out;
  out.reserve(in.size() + 2);
  for (char c : in) {
    switch (c) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if (static_cast<unsigned char>(c) < 0x20) {
          char buf[8];
          std::snprintf(buf, sizeof(buf), "\\u%04x", static_cast<int>(c));
          out += buf;
        } else {
          out += c;
        }
    }
  }
  return out;
}

void AppendField(std::string* out, const char* key, const std::string& v,
                 bool* first) {
  if (v.empty()) return;
  if (!*first) *out += ',';
  *first = false;
  *out += '"';
  *out += key;
  *out += "\":\"";
  *out += EscapeJson(v);
  *out += '"';
}

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
  std::string out = "{\"kind\":\"";
  out += TraceKindToString(kind);
  out += "\",\"ts_ms\":";
  char buf[32];
  std::snprintf(buf, sizeof(buf), "%lld", ts_ms);
  out += buf;
  bool first = false;  // ts_ms always present, others optional
  AppendField(&out, "space_id", space_id, &first);
  AppendField(&out, "run_id", run_id, &first);
  AppendField(&out, "agent_id", agent_id, &first);
  AppendField(&out, "tool_name", tool_name, &first);
  AppendField(&out, "doc_name", doc_name, &first);
  AppendField(&out, "doc_type", doc_type, &first);
  if (!payload_json.empty()) {
    out += ",\"payload\":";
    out += payload_json;
  }
  out += "}\n";
  return out;
}

}  // namespace cronymax
