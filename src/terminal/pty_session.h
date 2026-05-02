#pragma once

#include <atomic>
#include <filesystem>
#include <functional>
#include <string>
#include <thread>

#include <sys/types.h>

namespace cronymax {

class PtySession {
 public:
  using OutputCallback = std::function<void(std::string_view)>;
  using ExitCallback = std::function<void(int)>;

  PtySession();
  ~PtySession();

  PtySession(const PtySession&) = delete;
  PtySession& operator=(const PtySession&) = delete;

  bool Start(const std::filesystem::path& cwd,
             const std::string& shell,
             OutputCallback on_output,
             ExitCallback on_exit);
  void Write(std::string_view input);
  void Resize(int columns, int rows);
  void Stop();

  bool running() const { return running_; }
  pid_t pid() const { return child_pid_; }

 private:
  void ReadLoop();

  int master_fd_ = -1;
  pid_t child_pid_ = -1;
  std::atomic<bool> running_{false};
  std::thread reader_;
  OutputCallback on_output_;
  ExitCallback on_exit_;
};

}  // namespace cronymax

