#include "common/uuid_v7.h"

#include <chrono>
#include <cstdint>
#include <cstdio>
#include <mutex>
#include <random>

namespace cronymax {

namespace {

std::mutex& Mu() {
  static std::mutex m;
  return m;
}

std::mt19937_64& Rng() {
  static std::mt19937_64 rng{std::random_device{}()};
  return rng;
}

// Per-process monotonic state to keep ids strictly increasing across
// generators called within the same millisecond.
struct MonotonicState {
  long long last_ms = 0;
  std::uint16_t counter = 0;  // 12-bit "rand_a" slot
};

MonotonicState& State() {
  static MonotonicState s;
  return s;
}

std::string Format(long long ms, std::uint16_t rand_a, std::uint64_t rand_b) {
  // UUIDv7 layout (128 bits):
  //   unix_ts_ms : 48 bits (high)
  //   ver        : 4  bits = 0b0111
  //   rand_a     : 12 bits
  //   var        : 2  bits = 0b10
  //   rand_b     : 62 bits
  const std::uint64_t hi =
      (static_cast<std::uint64_t>(ms & 0xFFFFFFFFFFFFULL) << 16) |
      (0x7000ULL) |
      (static_cast<std::uint64_t>(rand_a & 0x0FFF));
  const std::uint64_t lo =
      (rand_b & 0x3FFFFFFFFFFFFFFFULL) |
      (0x8000000000000000ULL);  // variant 10xx
  // Format as 8-4-4-4-12 hex.
  char buf[40];
  std::snprintf(buf, sizeof(buf),
                "%08x-%04x-%04x-%04x-%012llx",
                static_cast<unsigned>((hi >> 32) & 0xFFFFFFFFULL),
                static_cast<unsigned>((hi >> 16) & 0xFFFFULL),
                static_cast<unsigned>(hi & 0xFFFFULL),
                static_cast<unsigned>((lo >> 48) & 0xFFFFULL),
                static_cast<unsigned long long>(lo & 0xFFFFFFFFFFFFULL));
  return std::string(buf);
}

long long NowMs() {
  using namespace std::chrono;
  return duration_cast<milliseconds>(system_clock::now().time_since_epoch())
      .count();
}

}  // namespace

std::string MakeUuidV7() {
  std::lock_guard<std::mutex> lock(Mu());
  long long ms = NowMs();
  auto& s = State();
  if (ms <= s.last_ms) {
    // Same (or earlier — clock-skew) millisecond: bump counter; if it
    // overflows, force ms to s.last_ms + 1 to preserve monotonic order.
    if (s.counter >= 0x0FFF) {
      ms = s.last_ms + 1;
      s.counter = 0;
    } else {
      ms = s.last_ms;
      s.counter += 1;
    }
  } else {
    s.counter = 0;
  }
  s.last_ms = ms;
  const std::uint64_t rand_b = Rng()();
  return Format(ms, s.counter, rand_b);
}

std::string MakeUuidV7At(long long unix_ms) {
  std::lock_guard<std::mutex> lock(Mu());
  const std::uint64_t rand_b = Rng()();
  return Format(unix_ms, /*rand_a=*/0, rand_b);
}

}  // namespace cronymax
