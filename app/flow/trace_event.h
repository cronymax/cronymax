#ifndef CRONYMAX_FLOW_TRACE_EVENT_H_
#define CRONYMAX_FLOW_TRACE_EVENT_H_

#include <string>

namespace cronymax {

// Typed kinds for the trace.jsonl stream. The kind is serialised as the
// `kind` field of every event line. Renderer subscribers can switch on it
// to render the correct UI affordance.
enum class TraceKind {
  kRunStarted,
  kRunCompleted,
  kRunCancelled,
  kRunFailed,
  kAgentStarted,
  kAgentEnded,
  kToolCall,
  kToolResult,
  kDocumentSubmitted,
  kReviewRequested,
  kReviewVerdict,
  kReviewExhausted,
  kRouted,
  kMention,
  kError,
};

const char* TraceKindToString(TraceKind k);

// One event in the trace stream. Fields are populated as appropriate for
// the kind; absent fields stay empty. ts_ms is unix-epoch milliseconds.
struct TraceEvent {
  TraceKind kind = TraceKind::kError;
  long long ts_ms = 0;
  std::string space_id;
  std::string run_id;
  std::string agent_id;
  std::string tool_name;
  std::string doc_name;
  std::string doc_type;
  // Free-form JSON payload (may be empty). Caller is responsible for it
  // being valid JSON — the writer interpolates it directly into the line.
  std::string payload_json;

  std::string ToJsonLine() const;  // includes trailing '\n'
};

}  // namespace cronymax

#endif  // CRONYMAX_FLOW_TRACE_EVENT_H_
