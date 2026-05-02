#include "event_bus/app_event.h"

#include <cstdio>

namespace cronymax::event_bus {

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

void AppendStr(std::string* out, const char* key, const std::string& v,
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

const char* AppEventKindToString(AppEventKind k) {
  switch (k) {
    case AppEventKind::kText: return "text";
    case AppEventKind::kAgentStatus: return "agent_status";
    case AppEventKind::kDocumentEvent: return "document_event";
    case AppEventKind::kReviewEvent: return "review_event";
    case AppEventKind::kHandoff: return "handoff";
    case AppEventKind::kError: return "error";
    case AppEventKind::kSystem: return "system";
  }
  return "system";
}

bool AppEventKindFromString(const std::string& s, AppEventKind* out) {
  if (s == "text") { *out = AppEventKind::kText; return true; }
  if (s == "agent_status") { *out = AppEventKind::kAgentStatus; return true; }
  if (s == "document_event") { *out = AppEventKind::kDocumentEvent; return true; }
  if (s == "review_event") { *out = AppEventKind::kReviewEvent; return true; }
  if (s == "handoff") { *out = AppEventKind::kHandoff; return true; }
  if (s == "error") { *out = AppEventKind::kError; return true; }
  if (s == "system") { *out = AppEventKind::kSystem; return true; }
  return false;
}

std::string AppEvent::ToJson() const {
  std::string out = "{\"id\":\"";
  out += EscapeJson(id);
  out += "\",\"ts_ms\":";
  char buf[32];
  std::snprintf(buf, sizeof(buf), "%lld", ts_ms);
  out += buf;
  out += ",\"kind\":\"";
  out += AppEventKindToString(kind);
  out += '"';
  bool first = false;
  AppendStr(&out, "space_id", space_id, &first);
  AppendStr(&out, "flow_id", flow_id, &first);
  AppendStr(&out, "run_id", run_id, &first);
  AppendStr(&out, "agent_id", agent_id, &first);
  out += ",\"payload\":";
  out += payload.is_object() ? payload.Dump(/*compact=*/true) : "{}";
  out += '}';
  return out;
}

bool AppEvent::ParseJson(const std::string& text, AppEvent* out,
                         std::string* err) {
  if (!out) return false;
  JsonValue v;
  std::string parse_err;
  if (!JsonValue::Parse(text, &v, &parse_err) || !v.is_object()) {
    if (err) *err = parse_err.empty() ? "not an object" : parse_err;
    return false;
  }
  const auto& id_v = v.Get("id");
  const auto& ts_v = v.Get("ts_ms");
  const auto& kind_v = v.Get("kind");
  if (!id_v.is_string() || !ts_v.is_number() || !kind_v.is_string()) {
    if (err) *err = "missing id/ts_ms/kind";
    return false;
  }
  AppEventKind k;
  if (!AppEventKindFromString(kind_v.as_string(), &k)) {
    if (err) *err = "unknown kind: " + kind_v.as_string();
    return false;
  }
  out->id = id_v.as_string();
  out->ts_ms = ts_v.as_i64();
  out->kind = k;
  auto load = [&](const char* key, std::string* dst) {
    const auto& jv = v.Get(key);
    if (jv.is_string()) *dst = jv.as_string();
    else dst->clear();
  };
  load("space_id", &out->space_id);
  load("flow_id", &out->flow_id);
  load("run_id", &out->run_id);
  load("agent_id", &out->agent_id);
  const auto& p = v.Get("payload");
  out->payload = p.is_object() ? p : JsonValue::Object();
  return true;
}

}  // namespace cronymax::event_bus
