#ifndef CRONYMAX_FLOW_FS_WATCHER_H_
#define CRONYMAX_FLOW_FS_WATCHER_H_

#include <chrono>
#include <filesystem>
#include <functional>
#include <memory>
#include <mutex>
#include <thread>
#include <vector>

namespace cronymax {

// Coalesced filesystem watcher backed by FSEvents on macOS. Reports a
// single notification per `debounce` window, regardless of how many file
// events occurred. The callback runs on a dedicated dispatch thread; it
// must be thread-safe relative to whatever data it touches.
//
// Used by the per-Space registries to reload .cronymax/agents/,
// .cronymax/flows/ and .cronymax/doc-types/ on disk changes.
class FsWatcher {
 public:
  using Callback = std::function<void()>;

  FsWatcher();
  ~FsWatcher();

  FsWatcher(const FsWatcher&) = delete;
  FsWatcher& operator=(const FsWatcher&) = delete;

  // Start watching `paths` recursively. `callback` is invoked at most once
  // per `debounce` interval after a burst of changes settles. Returns true
  // on success. Calling Start() while already running stops the previous
  // session first.
  bool Start(const std::vector<std::filesystem::path>& paths,
             std::chrono::milliseconds debounce, Callback callback);

  // Stop the watcher. Safe to call multiple times.
  void Stop();

  struct Impl;

 private:
  std::unique_ptr<Impl> impl_;
};

}  // namespace cronymax

#endif  // CRONYMAX_FLOW_FS_WATCHER_H_
