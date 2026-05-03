#include "event_bus/app_event.h"

#include <cstdio>

namespace cronymax::event_bus {

const char* AppEventKindToString(AppEventKind k) {
  switch (k) {
    case AppEventKind::kText:          return "text";
    case AppEventKind::kAgentStatus:   return "agent_status";
    case AppEventKind::kDocumentEvent: return "document_event";
    case AppEventKind::kReviewEvent:   return "review_event";
    case AppEventKind::kHandoff:       return "handoff";
    case AppEventKind::kError:         return "error";
    case AppEventKind::kSystem:        return "system";
  }
  return "system";
}

bool AppEventKindFromString(const std::string& s, AppEventKind* out) {
  if (s == "text")           { *out = AppEventKind::kText;          return true; }
  if (s == "agent_status")   { *out = AppEventKind::kAgentStatus;   return true; }
  if (s == "document_event") { *out = AppEventKind::kDocumentEvent; return true; }
  if (s == "review_event")   { *out = AppEventKind::kReviewEvent;   return true; }
  if (s == "handoff")        { *out = AppEventKind::kHandoff;       return true; }
  if (s == "error")          { *out = AppEventKind::kError;         return true; }
  if (s == "system")         { *out = AppEventKind::kSystem;        return true; }
  return false;
}

std::string AppEvent::ToJson() const {
  nlohmann::json j = {
    {"id",    id},
    {"ts_ms", ts_ms},
    {"kind",  AppEventKindToString(kind)},
  };
  if (!space_id.empty()) j["space_id"] = space_id;
  if (!flow_id.empty())  j["flow_id"]  = flow_id;
  if (!run_id.empty())   j["run_id"]   = run_id;
  if (!agent_id.empty()) j["agent_id"] = agent_id;
  j["payload"] = payload.is_object() ? payload : nlohmann::json::object();
  return j.dump();
}

bool AppEvent::ParseJson(const std::string& text, AppEvent* out,
                         std::string* err) {
  if (!out) return false;
  nlohmann::json v;
  v = nlohmann::json::parse(text, nullptr, false);
  if (v.is_discarded()) {
    if (err) *err = "JSON parse error";
    return false;
  }
  if (!v.is_object()) {
    if (err) *err = "not an object";
    return false;
  }
  if (!v.contains("id") || !v["id"].is_string() ||
      !v.contains("ts_ms") || !v["ts_ms"].is_number() ||
      !v.contains("kind") || !v["kind"].is_string()) {
    if (err) *err = "missing id/ts_ms/kind";
    return false;
  }
  AppEventKind k;
  if (!AppEventKindFromString(v["kind"].get<std::string>(), &k)) {
    if (err) *err = "unknown kind: " + v["kind"].get<std::string>();
    return false;
  }
  out->id    = v["id"].get<std::string>();
  out->ts_ms = v["ts_ms"].get<long long>();
  out->kind  = k;
  auto load = [&](const char* key, std::string* dst) {
    if (v.contains(key) && v[key].is_string())
      *dst = v[key].get<std::string>();
    else
      dst->clear();
  };
  load("space_id", &out->space_id);
  load("flow_id",  &out->flow_id);
  load("run_id",   &out->run_id);
  load("agent_id", &out->agent_id);
  out->payload = (v.contains("payload") && v["payload"].is_object())
                     ? v["payload"]
                     : nlohmann::json::object();
  return true;
}

}  // namespace cronymax::event_bus
