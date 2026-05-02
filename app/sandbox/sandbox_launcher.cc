#include "sandbox/sandbox_launcher.h"

#include <fcntl.h>
#include <poll.h>
#include <sys/wait.h>
#include <unistd.h>

#include <array>
#include <cerrno>
#include <cstring>
#include <fstream>
#include <sstream>
#include <vector>

#include "common/path_utils.h"

namespace cronymax {
namespace {

bool FileExists(const char* path) {
  return access(path, X_OK) == 0;
}

void SetNonBlocking(int fd) {
  const int flags = fcntl(fd, F_GETFL, 0);
  if (flags >= 0) {
    fcntl(fd, F_SETFL, flags | O_NONBLOCK);
  }
}

std::string ReadAvailable(int fd, bool& open) {
  std::string data;
  std::array<char, 4096> buffer{};

  while (true) {
    const ssize_t n = read(fd, buffer.data(), buffer.size());
    if (n > 0) {
      data.append(buffer.data(), static_cast<size_t>(n));
      continue;
    }
    if (n == 0) {
      open = false;
      close(fd);
      break;
    }
    if (errno == EAGAIN || errno == EWOULDBLOCK) {
      break;
    }
    open = false;
    close(fd);
    break;
  }

  return data;
}

ExecResult RunProcess(const std::vector<std::string>& argv,
                      const std::filesystem::path& cwd) {
  ExecResult result;
  int stdout_pipe[2] = {-1, -1};
  int stderr_pipe[2] = {-1, -1};

  if (pipe(stdout_pipe) != 0 || pipe(stderr_pipe) != 0) {
    result.stderr_data = "failed to create pipes";
    return result;
  }

  const pid_t pid = fork();
  if (pid < 0) {
    result.stderr_data = "failed to fork";
    close(stdout_pipe[0]);
    close(stdout_pipe[1]);
    close(stderr_pipe[0]);
    close(stderr_pipe[1]);
    return result;
  }

  if (pid == 0) {
    close(stdout_pipe[0]);
    close(stderr_pipe[0]);
    dup2(stdout_pipe[1], STDOUT_FILENO);
    dup2(stderr_pipe[1], STDERR_FILENO);
    close(stdout_pipe[1]);
    close(stderr_pipe[1]);

    if (!cwd.empty()) {
      chdir(cwd.c_str());
    }

    std::vector<char*> c_argv;
    c_argv.reserve(argv.size() + 1);
    for (const auto& arg : argv) {
      c_argv.push_back(const_cast<char*>(arg.c_str()));
    }
    c_argv.push_back(nullptr);

    execvp(c_argv[0], c_argv.data());
    _exit(127);
  }

  close(stdout_pipe[1]);
  close(stderr_pipe[1]);
  SetNonBlocking(stdout_pipe[0]);
  SetNonBlocking(stderr_pipe[0]);

  bool stdout_open = true;
  bool stderr_open = true;
  while (stdout_open || stderr_open) {
    std::array<pollfd, 2> fds{};
    int count = 0;
    if (stdout_open) {
      fds[count++] = {.fd = stdout_pipe[0], .events = POLLIN | POLLHUP};
    }
    if (stderr_open) {
      fds[count++] = {.fd = stderr_pipe[0], .events = POLLIN | POLLHUP};
    }

    const int rc = poll(fds.data(), count, 100);
    if (rc < 0 && errno != EINTR) {
      break;
    }

    int index = 0;
    if (stdout_open) {
      result.stdout_data += ReadAvailable(stdout_pipe[0], stdout_open);
      ++index;
    }
    if (stderr_open) {
      result.stderr_data += ReadAvailable(stderr_pipe[0], stderr_open);
      (void)index;
    }
  }

  int status = 0;
  if (waitpid(pid, &status, 0) >= 0) {
    if (WIFEXITED(status)) {
      result.exit_code = WEXITSTATUS(status);
    } else if (WIFSIGNALED(status)) {
      result.exit_code = 128 + WTERMSIG(status);
    }
  }

  return result;
}

std::filesystem::path WriteTempProfile(const std::string& profile,
                                       std::string& error) {
  char path_template[] = "/tmp/cronymax-sandbox-XXXXXX";
  const int fd = mkstemp(path_template);
  if (fd < 0) {
    error = "failed to create temporary sandbox profile";
    return {};
  }

  const ssize_t written = write(fd, profile.data(), profile.size());
  close(fd);

  if (written < 0 || static_cast<size_t>(written) != profile.size()) {
    unlink(path_template);
    error = "failed to write temporary sandbox profile";
    return {};
  }

  return path_template;
}

}  // namespace

ExecResult SandboxLauncher::ExecuteShellCommand(
    Actor actor,
    const SandboxPolicy& policy,
    const std::filesystem::path& cwd,
    const std::string& command,
    bool confirmation_granted) const {
  ExecResult denied;
  const auto decision = permission_broker_.CheckExec(actor, command, policy);
  if (!decision.allowed &&
      !(decision.requires_confirmation && confirmation_granted)) {
    denied.exit_code = 126;
    denied.stderr_data = "permission denied: " + decision.reason;
    for (const auto& reason : decision.risk_reasons) {
      denied.stderr_data += "\n- " + reason;
    }
    return denied;
  }

  if (!policy.CanRead(cwd) || !policy.CanWrite(cwd)) {
    denied.exit_code = 126;
    denied.stderr_data = "permission denied: cwd is outside workspace policy";
    return denied;
  }

  if (!FileExists("/usr/bin/sandbox-exec")) {
    denied.exit_code = 127;
    denied.stderr_data = "sandbox-exec is not available on this macOS host";
    return denied;
  }

  std::string profile_error;
  const auto profile_path =
      WriteTempProfile(policy.ToSeatbeltProfile(), profile_error);
  if (!profile_error.empty()) {
    denied.exit_code = 1;
    denied.stderr_data = profile_error;
    return denied;
  }

  const std::vector<std::string> argv = {
      "/usr/bin/sandbox-exec",
      "-f",
      profile_path.string(),
      "/bin/zsh",
      "-lc",
      command,
  };

  auto result = RunProcess(argv, NormalizePath(cwd));
  unlink(profile_path.c_str());
  return result;
}

}  // namespace cronymax

