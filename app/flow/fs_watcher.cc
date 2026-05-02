#include "flow/fs_watcher.h"

#include <CoreServices/CoreServices.h>

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <thread>
#include <utility>

namespace cronymax {

struct FsWatcher::Impl {
  // FSEvents state.
  FSEventStreamRef stream = nullptr;
  dispatch_queue_t queue = nullptr;

  // Debounce state.
  std::mutex mu;
  std::condition_variable cv;
  bool dirty = false;
  bool running = false;
  std::chrono::milliseconds debounce{0};
  Callback callback;
  std::thread debouncer;
};

namespace {

void OnFsEvent(ConstFSEventStreamRef /*streamRef*/, void* clientCallBackInfo,
               size_t /*numEvents*/, void* /*eventPaths*/,
               const FSEventStreamEventFlags* /*eventFlags*/,
               const FSEventStreamEventId* /*eventIds*/) {
  auto* impl = static_cast<FsWatcher::Impl*>(clientCallBackInfo);
  {
    std::lock_guard<std::mutex> lock(impl->mu);
    impl->dirty = true;
  }
  impl->cv.notify_one();
}

}  // namespace

FsWatcher::FsWatcher() : impl_(std::make_unique<Impl>()) {}

FsWatcher::~FsWatcher() { Stop(); }

bool FsWatcher::Start(const std::vector<std::filesystem::path>& paths,
                      std::chrono::milliseconds debounce, Callback callback) {
  Stop();

  if (paths.empty() || !callback) return false;

  impl_->debounce = debounce;
  impl_->callback = std::move(callback);
  impl_->dirty = false;
  impl_->running = true;

  CFMutableArrayRef cf_paths =
      CFArrayCreateMutable(nullptr, paths.size(), &kCFTypeArrayCallBacks);
  for (const auto& p : paths) {
    auto s = p.string();
    CFStringRef cf =
        CFStringCreateWithCString(nullptr, s.c_str(), kCFStringEncodingUTF8);
    if (cf) {
      CFArrayAppendValue(cf_paths, cf);
      CFRelease(cf);
    }
  }

  FSEventStreamContext ctx{0, impl_.get(), nullptr, nullptr, nullptr};
  // Latency is the FSEvents-internal coalescing window; we additionally
  // debounce in our own thread.
  const CFAbsoluteTime kFsLatency = 0.1;  // 100 ms
  impl_->stream = FSEventStreamCreate(
      nullptr, &OnFsEvent, &ctx, cf_paths,
      kFSEventStreamEventIdSinceNow, kFsLatency,
      kFSEventStreamCreateFlagFileEvents);
  CFRelease(cf_paths);

  if (!impl_->stream) {
    impl_->running = false;
    return false;
  }

  impl_->queue =
      dispatch_queue_create("com.cronymax.fswatcher", DISPATCH_QUEUE_SERIAL);
  FSEventStreamSetDispatchQueue(impl_->stream, impl_->queue);
  FSEventStreamStart(impl_->stream);

  // Debouncer: when dirty, wait `debounce`, then fire callback.
  impl_->debouncer = std::thread([impl = impl_.get()]() {
    while (true) {
      std::unique_lock<std::mutex> lock(impl->mu);
      impl->cv.wait(lock, [&] { return impl->dirty || !impl->running; });
      if (!impl->running) return;
      // Settle: keep extending the wait while new events arrive.
      while (impl->dirty && impl->running) {
        impl->dirty = false;
        impl->cv.wait_for(lock, impl->debounce);
      }
      if (!impl->running) return;
      lock.unlock();
      // Run callback outside the lock.
      try {
        impl->callback();
      } catch (...) {
        // Swallow — registry refresh shouldn't take down the watcher.
      }
    }
  });

  return true;
}

void FsWatcher::Stop() {
  if (impl_->stream) {
    FSEventStreamStop(impl_->stream);
    FSEventStreamInvalidate(impl_->stream);
    FSEventStreamRelease(impl_->stream);
    impl_->stream = nullptr;
  }
  if (impl_->queue) {
    dispatch_release(impl_->queue);
    impl_->queue = nullptr;
  }
  {
    std::lock_guard<std::mutex> lock(impl_->mu);
    impl_->running = false;
  }
  impl_->cv.notify_all();
  if (impl_->debouncer.joinable()) impl_->debouncer.join();
  impl_->callback = nullptr;
}

}  // namespace cronymax
