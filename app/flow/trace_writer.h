#ifndef CRONYMAX_FLOW_TRACE_WRITER_H_
#define CRONYMAX_FLOW_TRACE_WRITER_H_

#include <atomic>
#include <condition_variable>
#include <deque>
#include <filesystem>
#include <functional>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "flow/trace_event.h"

namespace cronymax {

// Append-only writer for `runs/<id>/trace.jsonl`. Calls to Append are
// non-blocking: events are queued and flushed on a background thread.
//
// Subscribers can register a callback to receive every event after it has
// been appended (this is what powers the renderer's `event` stream — the
// SubscribeReplay() helper supports replay-then-live by replaying the
// existing file then attaching the live callback under the same lock).
//
// Thread-safety: public methods are mutex-protected.
class TraceWriter {
 public:
  using Subscriber = std::function<void(const TraceEvent& evt)>;

  // `trace_path` is the absolute file path of trace.jsonl.
  explicit TraceWriter(std::filesystem::path trace_path);
  ~TraceWriter();

  TraceWriter(const TraceWriter&) = delete;
  TraceWriter& operator=(const TraceWriter&) = delete;

  // Enqueue an event for write + subscriber dispatch. Non-blocking.
  void Append(TraceEvent evt);

  // Block until the queue drains. Used by tests and on shutdown.
  void Flush();

  // Subscribe with replay: invokes `cb` synchronously for every event
  // already on disk, then keeps it attached for live events. Returns a
  // token used to Unsubscribe; tokens are positive.
  std::size_t SubscribeReplay(Subscriber cb);
  void Unsubscribe(std::size_t token);

 private:
  void WriterLoop();
  void Notify(const TraceEvent& evt);

  std::filesystem::path path_;
  std::thread thread_;
  std::mutex mu_;
  std::condition_variable cv_;
  std::deque<TraceEvent> queue_;
  std::vector<std::pair<std::size_t, Subscriber>> subs_;
  std::size_t next_token_ = 1;
  std::atomic<bool> stop_{false};
};

}  // namespace cronymax

#endif  // CRONYMAX_FLOW_TRACE_WRITER_H_
