#include "terminal/pty_session.h"

#include <fcntl.h>
#include <signal.h>
#include <stdlib.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <unistd.h>
#include <util.h>

#include <array>
#include <string>

namespace cronymax {

PtySession::PtySession() = default;

PtySession::~PtySession() {
  Stop();
}

bool PtySession::Start(const std::filesystem::path& cwd,
                       const std::string& shell,
                       OutputCallback on_output,
                       ExitCallback on_exit) {
  if (running_) {
    return false;
  }

  on_output_ = std::move(on_output);
  on_exit_ = std::move(on_exit);

  winsize size{};
  size.ws_col = 100;
  size.ws_row = 30;

  child_pid_ = forkpty(&master_fd_, nullptr, nullptr, &size);
  if (child_pid_ < 0) {
    master_fd_ = -1;
    child_pid_ = -1;
    return false;
  }

  if (child_pid_ == 0) {
    chdir(cwd.c_str());
    setenv("TERM", "xterm-256color", 1);
    setenv("AI_DESKTOP_TERMINAL", "1", 1);

    // OSC 133 shell integration: inject precmd/preexec hooks via ZDOTDIR.
    // We write a minimal .zshrc to a tmp dir and point ZDOTDIR at it so that
    // the real user .zshrc is not disturbed.
    char zdot_tmpl[] = "/tmp/cronymax_zdot_XXXXXX";
    if (char* zdot = mkdtemp(zdot_tmpl)) {
      std::string rc_path = std::string(zdot) + "/.zshrc";
      // Source the real ~/.zshrc first (if it exists), then install hooks.
      const char* home = getenv("HOME");
      std::string rc_content;
      if (home) {
        rc_content += "[ -f \"" + std::string(home) + "/.zshrc\" ] && source \"" +
                      std::string(home) + "/.zshrc\"\n";
      }
      rc_content +=
          "# AI Desktop shell integration (OSC 133 + OSC 7)\n"
          "function _ai_preexec() {\n"
          "  printf '\\033]133;C\\007'\n"
          "  export _AI_CMD_START=$SECONDS\n"
          "}\n"
          "function _ai_precmd() {\n"
          "  local ec=$?\n"
          "  printf \"\\033]133;D;%d\\007\" $ec\n"
          "}\n"
          "# OSC 7: emit CWD on every directory change and on startup\n"
          "function _ai_cwd() {\n"
          "  printf '\\033]7;file://%s%s\\007' \"$HOST\" \"$PWD\"\n"
          "}\n"
          "autoload -Uz add-zsh-hook\n"
          "add-zsh-hook preexec _ai_preexec\n"
          "add-zsh-hook precmd  _ai_precmd\n"
          "add-zsh-hook chpwd   _ai_cwd\n"
          "_ai_cwd\n";  // emit CWD immediately so title bar is populated
      int fd = open(rc_path.c_str(), O_WRONLY | O_CREAT | O_TRUNC, 0600);
      if (fd >= 0) {
        write(fd, rc_content.c_str(), rc_content.size());
        close(fd);
        setenv("ZDOTDIR", zdot, 1);
      }
    }

    execl(shell.c_str(), shell.c_str(), "-l", nullptr);
    _exit(127);
  }

  running_ = true;
  reader_ = std::thread([this] { ReadLoop(); });
  return true;
}

void PtySession::Write(std::string_view input) {
  if (!running_ || master_fd_ < 0) {
    return;
  }
  (void)write(master_fd_, input.data(), input.size());
}

void PtySession::Resize(int columns, int rows) {
  if (!running_ || master_fd_ < 0) {
    return;
  }

  winsize size{};
  size.ws_col = static_cast<unsigned short>(columns);
  size.ws_row = static_cast<unsigned short>(rows);
  ioctl(master_fd_, TIOCSWINSZ, &size);
}

void PtySession::Stop() {
  if (!running_ && master_fd_ < 0 && child_pid_ <= 0) {
    return;
  }

  running_ = false;

  if (master_fd_ >= 0) {
    close(master_fd_);
    master_fd_ = -1;
  }

  if (child_pid_ > 0) {
    kill(child_pid_, SIGHUP);
  }

  if (reader_.joinable()) {
    reader_.join();
  }

  if (child_pid_ > 0) {
    int status = 0;
    waitpid(child_pid_, &status, WNOHANG);
    child_pid_ = -1;
  }
}

void PtySession::ReadLoop() {
  std::array<char, 4096> buffer{};
  while (running_) {
    const ssize_t n = read(master_fd_, buffer.data(), buffer.size());
    if (n > 0) {
      if (on_output_) {
        on_output_(std::string_view(buffer.data(), static_cast<size_t>(n)));
      }
      continue;
    }
    break;
  }

  running_ = false;

  int exit_code = -1;
  if (child_pid_ > 0) {
    int status = 0;
    if (waitpid(child_pid_, &status, WNOHANG) > 0) {
      if (WIFEXITED(status)) {
        exit_code = WEXITSTATUS(status);
      } else if (WIFSIGNALED(status)) {
        exit_code = 128 + WTERMSIG(status);
      }
    }
  }

  if (on_exit_) {
    on_exit_(exit_code);
  }
}

}  // namespace cronymax

