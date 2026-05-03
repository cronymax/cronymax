#include "flow/trace_writer.h"

#include <fstream>
#include <sstream>
#include <system_error>

#include <nlohmann/json.hpp>

namespace cronymax {

namespace {

namespace fs = std::filesystem;

// Reconstruct a TraceEvent from a previously serialised line. We only
// need this for replay-on-subscribe, so it's minimal: we carry the raw
// JSON line through as payload_json so the renderer sees it unchanged.
TraceEvent ParseLine(const std::string& line) {
  TraceEvent evt;
  nlohmann::json v;
  v = nlohmann::json::parse(line, nullptr, false);
  if (v.is_discarded() || !v.is_object()) {
    evt.kind = TraceKind::kError;
    evt.payload_json = line;
    return evt;
  }
  if (v.contains("kind") && v["kind"].is_string()) {
    const auto& s = v["kind"].get<std::string>();
    // Map back via linear scan (small enum).
    if (s == "run.started")          evt.kind = TraceKind::kRunStarted;
    else if (s == "run.completed")   evt.kind = TraceKind::kRunCompleted;
    else if (s == "run.cancelled")   evt.kind = TraceKind::kRunCancelled;
    else if (s == "run.failed")      evt.kind = TraceKind::kRunFailed;
    else if (s == "agent.started")   evt.kind = TraceKind::kAgentStarted;
    else if (s == "agent.ended")     evt.kind = TraceKind::kAgentEnded;
    else if (s == "tool.call")       evt.kind = TraceKind::kToolCall;
    else if (s == "tool.result")     evt.kind = TraceKind::kToolResult;
    else if (s == "document.submitted") evt.kind = TraceKind::kDocumentSubmitted;
    else if (s == "review.requested") evt.kind = TraceKind::kReviewRequested;
    else if (s == "review.verdict")  evt.kind = TraceKind::kReviewVerdict;
    else if (s == "review.exhausted") evt.kind = TraceKind::kReviewExhausted;
    else if (s == "routed")          evt.kind = TraceKind::kRouted;
    else if (s == "mention")         evt.kind = TraceKind::kMention;
    else                             evt.kind = TraceKind::kError;
  }
  if (v.contains("ts_ms") && v["ts_ms"].is_number())
    evt.ts_ms = v["ts_ms"].get<long long>();
  auto load_str = [&](const char* k, std::string* dst) {
    if (v.contains(k) && v[k].is_string()) *dst = v[k].get<std::string>();
  };
  load_str("space_id",  &evt.space_id);
  load_str("run_id",    &evt.run_id);
  load_str("agent_id",  &evt.agent_id);
  load_str("tool_name", &evt.tool_name);
  load_str("doc_name",  &evt.doc_name);
  load_str("doc_type",  &evt.doc_type);
  // Whole line is the payload for renderer convenience.
  evt.payload_json = line;
  return evt;
}

}  // namespace

TraceWriter::TraceWriter(fs::path trace_path) : path_(std::move(trace_path)) {
  std::error_code ec;
  fs::create_directories(path_.parent_path(), ec);
  thread_ = std::thread(&TraceWriter::WriterLoop, this);
}

TraceWriter::~TraceWriter() {
  stop_.store(true);
  cv_.notify_all();
  if (thread_.joinable()) thread_.join();
}

void TraceWriter::Append(TraceEvent evt) {
  std::lock_guard<std::mutex> lock(mu_);
  queue_.push_back(std::move(evt));
  cv_.notify_one();
}

void TraceWriter::Flush() {
  std::unique_lock<std::mutex> lock(mu_);
  cv_.wait(lock, [&] { return queue_.empty(); });
}

std::size_t TraceWriter::SubscribeReplay(Subscriber cb) {
  std::lock_guard<std::mutex> lock(mu_);
  // Replay file under the same lock so no live event slips between
  // replay and live attachment (preserves total order per design).
  std::ifstream in(path_, std::ios::binary);
  std::string line;
  while (std::getline(in, line)) {
    if (line.empty()) continue;
    cb(ParseLine(line));
  }
  const auto token = next_token_++;
  subs_.push_back({token, std::move(cb)});
  return token;
}

void TraceWriter::Unsubscribe(std::size_t token) {
  std::lock_guard<std::mutex> lock(mu_);
  for (auto it = subs_.begin(); it != subs_.end(); ++it) {
    if (it->first == token) {
      subs_.erase(it);
      return;
    }
  }
}

void TraceWriter::Notify(const TraceEvent& evt) {
  // Caller must hold mu_.
  for (const auto& kv : subs_) kv.second(evt);
}

void TraceWriter::WriterLoop() {
  while (true) {
    TraceEvent evt;
    {
      std::unique_lock<std::mutex> lock(mu_);
      cv_.wait(lock,
               [&] { return stop_.load() || !queue_.empty(); });
      if (queue_.empty()) {
        if (stop_.load()) return;
        continue;
      }
      evt = std::move(queue_.front());
      queue_.pop_front();
      // Append + dispatch under the same lock so subscribers see events
      // in the same order as the file.
      std::ofstream out(path_, std::ios::binary | std::ios::app);
      if (out) {
        const auto line = evt.ToJsonLine();
        out.write(line.data(), static_cast<std::streamsize>(line.size()));
      }
      Notify(evt);
      if (queue_.empty()) cv_.notify_all();  // wake Flush()
    }
  }
}

}  // namespace cronymax
