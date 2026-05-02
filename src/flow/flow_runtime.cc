#include "flow/flow_runtime.h"

#include <chrono>
#include <cstdio>
#include <ctime>
#include <fstream>
#include <random>
#include <sstream>
#include <system_error>

#include "agent/agent_runtime.h"
#include "common/json_value.h"
#include "document/agent_registry.h"
#include "document/doc_type_registry.h"
#include "document/document_store.h"
#include "document/review_store.h"
#include "event_bus/app_event.h"
#include "event_bus/event_bus.h"
#include "flow/flow_definition.h"
#include "flow/flow_registry.h"
#include "flow/trace_event.h"
#include "flow/trace_writer.h"

namespace cronymax {

namespace {

// Build a `kind=system` AppEvent describing a run-lifecycle transition.
// `subkind` is one of "run_started", "run_completed", "run_cancelled".
event_bus::AppEvent MakeSystemRunEvent(const std::string& subkind,
                                       const std::string& space_id,
                                       const std::string& run_id,
                                       const std::string& flow_id,
                                       const std::string& agent_id) {
  event_bus::AppEvent evt;
  evt.kind = event_bus::AppEventKind::kSystem;
  evt.space_id = space_id;
  evt.run_id = run_id;
  evt.flow_id = flow_id;
  evt.agent_id = agent_id;
  JsonValue payload = JsonValue::Object();
  payload.as_object()["subkind"] = JsonValue::String(subkind);
  evt.payload = std::move(payload);
  return evt;
}

namespace fs = std::filesystem;

std::string IsoNowUtc() {
  const auto now = std::chrono::system_clock::now();
  const auto t = std::chrono::system_clock::to_time_t(now);
  std::tm tm_utc{};
  gmtime_r(&t, &tm_utc);
  char buf[32];
  std::strftime(buf, sizeof(buf), "%Y-%m-%dT%H:%M:%SZ", &tm_utc);
  return buf;
}

}  // namespace

std::string FlowRunStatusToString(FlowRunStatus s) {
  switch (s) {
    case FlowRunStatus::kPending: return "PENDING";
    case FlowRunStatus::kRunning: return "RUNNING";
    case FlowRunStatus::kPaused: return "PAUSED";
    case FlowRunStatus::kCompleted: return "COMPLETED";
    case FlowRunStatus::kCancelled: return "CANCELLED";
    case FlowRunStatus::kFailed: return "FAILED";
  }
  return "PENDING";
}

FlowRunStatus ParseFlowRunStatus(const std::string& s) {
  if (s == "RUNNING") return FlowRunStatus::kRunning;
  if (s == "PAUSED") return FlowRunStatus::kPaused;
  if (s == "COMPLETED") return FlowRunStatus::kCompleted;
  if (s == "CANCELLED") return FlowRunStatus::kCancelled;
  if (s == "FAILED") return FlowRunStatus::kFailed;
  return FlowRunStatus::kPending;
}

// ---------------------------------------------------------------------------
// FlowRunState JSON
// ---------------------------------------------------------------------------

std::string FlowRunState::ToJson() const {
  JsonValue root = JsonValue::Object();
  auto& o = root.as_object();
  o["run_id"] = JsonValue::String(run_id);
  o["flow_id"] = JsonValue::String(flow_id);
  o["status"] = JsonValue::String(FlowRunStatusToString(status));
  o["started_at"] = JsonValue::String(started_at_iso);
  o["ended_at"] = JsonValue::String(ended_at_iso);
  o["failure_reason"] = JsonValue::String(failure_reason);
  o["initial_input"] = JsonValue::String(initial_input);

  JsonValue agents = JsonValue::Array();
  for (const auto& a : agents_in_flight) {
    agents.as_array().push_back(JsonValue::String(a));
  }
  o["agents_in_flight"] = agents;

  JsonValue docs = JsonValue::Array();
  for (const auto& d : documents) {
    JsonValue obj = JsonValue::Object();
    auto& dom = obj.as_object();
    dom["name"] = JsonValue::String(d.name);
    dom["type"] = JsonValue::String(d.type);
    dom["producer_agent"] = JsonValue::String(d.producer_agent);
    dom["current_revision"] =
        JsonValue::Number(static_cast<double>(d.current_revision));
    docs.as_array().push_back(obj);
  }
  o["documents"] = docs;

  return root.Dump(true);
}

bool FlowRunState::FromJson(const std::string& json, FlowRunState* out,
                            std::string* err) {
  JsonValue v;
  std::string parse_err;
  if (!JsonValue::Parse(json, &v, &parse_err)) {
    if (err) *err = "parse: " + parse_err;
    return false;
  }
  if (!v.is_object()) {
    if (err) *err = "expected object";
    return false;
  }
  auto str_field = [&](const char* key, std::string* dst) {
    const auto& jv = v.Get(key);
    if (jv.is_string()) *dst = jv.as_string();
  };
  str_field("run_id", &out->run_id);
  str_field("flow_id", &out->flow_id);
  str_field("started_at", &out->started_at_iso);
  str_field("ended_at", &out->ended_at_iso);
  str_field("failure_reason", &out->failure_reason);
  str_field("initial_input", &out->initial_input);
  if (const auto& s = v.Get("status"); s.is_string()) {
    out->status = ParseFlowRunStatus(s.as_string());
  }
  if (const auto& arr = v.Get("agents_in_flight"); arr.is_array()) {
    for (const auto& el : arr.as_array()) {
      if (el.is_string()) out->agents_in_flight.push_back(el.as_string());
    }
  }
  if (const auto& arr = v.Get("documents"); arr.is_array()) {
    for (const auto& el : arr.as_array()) {
      if (!el.is_object()) continue;
      FlowRunDocumentEntry entry;
      if (const auto& s = el.Get("name"); s.is_string()) entry.name = s.as_string();
      if (const auto& s = el.Get("type"); s.is_string()) entry.type = s.as_string();
      if (const auto& s = el.Get("producer_agent"); s.is_string()) {
        entry.producer_agent = s.as_string();
      }
      if (const auto& n = el.Get("current_revision"); n.is_number()) {
        entry.current_revision = static_cast<int>(n.as_number());
      }
      out->documents.push_back(std::move(entry));
    }
  }
  return true;
}

// ---------------------------------------------------------------------------
// FlowRuntime
// ---------------------------------------------------------------------------

FlowRuntime::FlowRuntime(fs::path workspace_root,
                         FlowRegistry* flow_registry,
                         AgentRegistry* agent_registry,
                         DocTypeRegistry* doc_type_registry)
    : workspace_root_(std::move(workspace_root)),
      flow_registry_(flow_registry),
      agent_registry_(agent_registry),
      doc_type_registry_(doc_type_registry) {}

FlowRuntime::~FlowRuntime() = default;

void FlowRuntime::SetEventEmitter(EventEmitter cb) {
  std::lock_guard<std::mutex> lock(mu_);
  emitter_ = std::move(cb);
}

void FlowRuntime::SetSpaceId(std::string id) {
  std::lock_guard<std::mutex> lock(mu_);
  space_id_ = std::move(id);
}

void FlowRuntime::Emit(const std::string& event,
                       const std::string& json) const {
  // Caller must hold mu_ OR ensure emitter_ is stable.
  if (emitter_) emitter_(event, json);
}

std::string FlowRuntime::GenerateRunId() const {
  // Format: r-<unix-ms>-<4 random hex>
  const auto now = std::chrono::system_clock::now().time_since_epoch();
  const auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(now)
                      .count();
  thread_local std::mt19937_64 rng{std::random_device{}()};
  std::uniform_int_distribution<uint32_t> dist(0, 0xffff);
  char buf[64];
  std::snprintf(buf, sizeof(buf), "r-%lld-%04x",
                static_cast<long long>(ms), dist(rng));
  return buf;
}

fs::path FlowRuntime::RunDir(const std::string& flow_id,
                             const std::string& run_id) const {
  return workspace_root_ / ".cronymax" / "flows" / flow_id / "runs" / run_id;
}

bool FlowRuntime::PersistState(const Run& run, std::string* err) const {
  std::error_code ec;
  fs::create_directories(run.run_dir, ec);
  if (ec) {
    if (err) *err = "create_directories: " + ec.message();
    return false;
  }
  const fs::path tmp = run.run_dir / "state.json.tmp";
  const fs::path target = run.run_dir / "state.json";
  {
    std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
    if (!out) {
      if (err) *err = "open tmp failed";
      return false;
    }
    out << run.state->ToJson();
    if (!out.good()) {
      if (err) *err = "write failed";
      return false;
    }
  }
  fs::rename(tmp, target, ec);
  if (ec) {
    if (err) *err = "rename: " + ec.message();
    return false;
  }
  return true;
}

int FlowRuntime::RehydrateFromDisk() {
  std::lock_guard<std::mutex> lock(mu_);
  const fs::path flows_root = workspace_root_ / ".cronymax" / "flows";
  std::error_code ec;
  if (!fs::exists(flows_root, ec)) return 0;
  int paused = 0;
  for (const auto& flow_entry : fs::directory_iterator(flows_root, ec)) {
    if (!flow_entry.is_directory()) continue;
    const fs::path runs_root = flow_entry.path() / "runs";
    if (!fs::exists(runs_root, ec)) continue;
    for (const auto& run_entry : fs::directory_iterator(runs_root, ec)) {
      if (!run_entry.is_directory()) continue;
      const fs::path state_path = run_entry.path() / "state.json";
      if (!fs::exists(state_path, ec)) continue;
      std::ifstream in(state_path, std::ios::binary);
      if (!in) continue;
      std::stringstream ss; ss << in.rdbuf();
      auto state = std::make_shared<FlowRunState>();
      std::string err;
      if (!FlowRunState::FromJson(ss.str(), state.get(), &err)) continue;
      // Per design Decision 7: any RUNNING state from a prior process is
      // PAUSED on rehydration; user must explicitly resume.
      if (state->status == FlowRunStatus::kRunning) {
        state->status = FlowRunStatus::kPaused;
      }
      Run run;
      run.state = state;
      run.run_dir = run_entry.path();
      run.doc_store = std::make_shared<DocumentStore>(
          workspace_root_ / ".cronymax" / "flows" / state->flow_id);
      run.review_store = std::make_shared<ReviewStore>(run.run_dir);
      run.trace_writer = std::make_shared<TraceWriter>(
          run.run_dir / "trace.jsonl");
      runs_[state->run_id] = std::move(run);
      if (state->status == FlowRunStatus::kPaused) ++paused;
    }
  }
  return paused;
}

std::string FlowRuntime::StartRun(const std::string& flow_id,
                                  const std::string& initial_input,
                                  std::string* err) {
  if (!flow_registry_) {
    if (err) *err = "no flow registry";
    return std::string();
  }
  const FlowDefinition* def = flow_registry_->Get(flow_id);
  if (!def) {
    if (err) *err = "flow not found: " + flow_id;
    return std::string();
  }
  if (def->agents().empty()) {
    if (err) *err = "flow has no agents";
    return std::string();
  }

  const std::string run_id = GenerateRunId();
  const fs::path run_dir = RunDir(flow_id, run_id);

  auto state = std::make_shared<FlowRunState>();
  state->run_id = run_id;
  state->flow_id = flow_id;
  state->status = FlowRunStatus::kRunning;
  state->started_at_iso = IsoNowUtc();
  state->initial_input = initial_input;
  // Entry agent = first declared agent (per spec Decision 4).
  const std::string& entry_agent = def->agents().front();
  state->agents_in_flight.push_back(entry_agent);

  Run run;
  run.state = state;
  run.run_dir = run_dir;
  run.doc_store = std::make_shared<DocumentStore>(
      workspace_root_ / ".cronymax" / "flows" / flow_id);
  run.review_store = std::make_shared<ReviewStore>(run_dir);
  // Trace writer attached up-front so submit_document/etc emit into it.
  std::error_code mkec;
  fs::create_directories(run_dir, mkec);
  run.trace_writer = std::make_shared<TraceWriter>(run_dir / "trace.jsonl");

  // Spin up an AgentRuntime for the entry agent. Downstream agents are
  // instantiated lazily as routing decisions are made by the renderer's
  // ReAct loop (or a future native router).
  AgentIdentity ident;
  ident.agent_id = entry_agent;
  ident.flow_run_id = run_id;
  ident.memory_namespace = entry_agent;
  FlowBindings bindings;
  bindings.flow_id = flow_id;
  bindings.document_store = run.doc_store;
  bindings.review_store = run.review_store;
  // Determine producing port for this agent from edges: the first edge
  // where from_agent matches.
  for (const auto& e : def->edges()) {
    if (e.from_agent == entry_agent) {
      bindings.producing_type = e.port;
      break;
    }
  }
  auto agent = std::make_shared<AgentRuntime>(workspace_root_, ident, bindings);
  run.agents[entry_agent] = agent;

  std::string persist_err;
  if (!PersistState(run, &persist_err)) {
    if (err) *err = "persist: " + persist_err;
    return std::string();
  }

  std::string event_json;
  std::shared_ptr<TraceWriter> tw_local;
  std::string flow_id_local;
  {
    std::lock_guard<std::mutex> lock(mu_);
    tw_local = run.trace_writer;
    flow_id_local = run.state->flow_id;
    runs_[run_id] = std::move(run);
    event_json = state->ToJson();
  }
  // Emit run.started: prefer EventBus when wired (production); fall back
  // to legacy TraceWriter for tests/loader_test that lack a Space.
  if (event_bus_) {
    event_bus_->Append(MakeSystemRunEvent(
        "run_started", space_id_, run_id, flow_id_local, entry_agent));
  } else if (tw_local) {
    TraceEvent evt;
    evt.kind = TraceKind::kRunStarted;
    evt.ts_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()).count();
    evt.space_id = space_id_;
    evt.run_id = run_id;
    evt.agent_id = entry_agent;
    tw_local->Append(std::move(evt));
  }
  Emit("flow.run.changed", event_json);
  return run_id;
}

bool FlowRuntime::CancelRun(const std::string& run_id, std::string* err) {
  std::shared_ptr<FlowRunState> state_copy;
  std::shared_ptr<TraceWriter> tw_local;
  std::string flow_id_local;
  {
    std::lock_guard<std::mutex> lock(mu_);
    auto it = runs_.find(run_id);
    if (it == runs_.end()) {
      if (err) *err = "unknown run_id";
      return false;
    }
    auto& run = it->second;
    const auto s = run.state->status;
    if (s == FlowRunStatus::kCompleted || s == FlowRunStatus::kCancelled ||
        s == FlowRunStatus::kFailed) {
      // Already terminal: idempotent success.
      return true;
    }
    run.state->status = FlowRunStatus::kCancelled;
    run.state->ended_at_iso = IsoNowUtc();
    run.state->agents_in_flight.clear();
    std::string perr;
    if (!PersistState(run, &perr)) {
      if (err) *err = perr;
      return false;
    }
    state_copy = std::make_shared<FlowRunState>(*run.state);
    tw_local = run.trace_writer;
    flow_id_local = run.state->flow_id;
  }
  if (event_bus_) {
    event_bus_->Append(MakeSystemRunEvent(
        "run_cancelled", space_id_, run_id, flow_id_local, std::string()));
  } else if (tw_local) {
    TraceEvent evt;
    evt.kind = TraceKind::kRunCancelled;
    evt.ts_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()).count();
    evt.space_id = space_id_;
    evt.run_id = run_id;
    tw_local->Append(std::move(evt));
  }
  Emit("flow.run.changed", state_copy->ToJson());
  return true;
}

bool FlowRuntime::CompleteRun(const std::string& run_id, std::string* err) {
  std::shared_ptr<FlowRunState> state_copy;
  std::shared_ptr<TraceWriter> tw_local;
  std::string flow_id_local;
  {
    std::lock_guard<std::mutex> lock(mu_);
    auto it = runs_.find(run_id);
    if (it == runs_.end()) {
      if (err) *err = "unknown run_id";
      return false;
    }
    auto& run = it->second;
    run.state->status = FlowRunStatus::kCompleted;
    run.state->ended_at_iso = IsoNowUtc();
    run.state->agents_in_flight.clear();
    std::string perr;
    if (!PersistState(run, &perr)) {
      if (err) *err = perr;
      return false;
    }
    state_copy = std::make_shared<FlowRunState>(*run.state);
    tw_local = run.trace_writer;
    flow_id_local = run.state->flow_id;
  }
  if (event_bus_) {
    event_bus_->Append(MakeSystemRunEvent(
        "run_completed", space_id_, run_id, flow_id_local, std::string()));
  } else if (tw_local) {
    TraceEvent evt;
    evt.kind = TraceKind::kRunCompleted;
    evt.ts_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()).count();
    evt.space_id = space_id_;
    evt.run_id = run_id;
    tw_local->Append(std::move(evt));
  }
  Emit("flow.run.changed", state_copy->ToJson());
  return true;
}

std::shared_ptr<FlowRunState> FlowRuntime::GetRun(
    const std::string& run_id) const {
  std::lock_guard<std::mutex> lock(mu_);
  auto it = runs_.find(run_id);
  if (it == runs_.end()) return nullptr;
  return it->second.state;
}

std::vector<std::shared_ptr<FlowRunState>> FlowRuntime::ListRuns() const {
  std::lock_guard<std::mutex> lock(mu_);
  std::vector<std::shared_ptr<FlowRunState>> out;
  out.reserve(runs_.size());
  for (const auto& kv : runs_) out.push_back(kv.second.state);
  return out;
}

std::shared_ptr<AgentRuntime> FlowRuntime::GetAgent(
    const std::string& run_id, const std::string& agent_id) const {
  std::lock_guard<std::mutex> lock(mu_);
  auto it = runs_.find(run_id);
  if (it == runs_.end()) return nullptr;
  auto ait = it->second.agents.find(agent_id);
  if (ait == it->second.agents.end()) return nullptr;
  return ait->second;
}

std::shared_ptr<DocumentStore> FlowRuntime::GetDocumentStore(
    const std::string& run_id) const {
  std::lock_guard<std::mutex> lock(mu_);
  auto it = runs_.find(run_id);
  if (it == runs_.end()) return nullptr;
  return it->second.doc_store;
}

std::shared_ptr<ReviewStore> FlowRuntime::GetReviewStore(
    const std::string& run_id) const {
  std::lock_guard<std::mutex> lock(mu_);
  auto it = runs_.find(run_id);
  if (it == runs_.end()) return nullptr;
  return it->second.review_store;
}

std::shared_ptr<TraceWriter> FlowRuntime::GetTraceWriter(
    const std::string& run_id) const {
  std::lock_guard<std::mutex> lock(mu_);
  auto it = runs_.find(run_id);
  if (it == runs_.end()) return nullptr;
  return it->second.trace_writer;
}

}  // namespace cronymax
