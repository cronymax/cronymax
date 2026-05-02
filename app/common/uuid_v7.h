#ifndef CRONYMAX_COMMON_UUID_V7_H_
#define CRONYMAX_COMMON_UUID_V7_H_

#include <string>

namespace cronymax {

// Generate a UUIDv7 string in 8-4-4-4-12 hex form. UUIDv7 embeds a
// big-endian 48-bit Unix-millisecond timestamp in its high bits, so the
// lexicographic ordering of the textual UUID matches chronological
// ordering for events appended within the same millisecond cohort. We
// add a per-process monotonic counter into the rand_a slot to keep ids
// strictly increasing within a single millisecond. Thread-safe.
std::string MakeUuidV7();

// Optional: deterministic test variant — caller supplies the timestamp.
// Useful in tests that need reproducible ordering.
std::string MakeUuidV7At(long long unix_ms);

}  // namespace cronymax

#endif  // CRONYMAX_COMMON_UUID_V7_H_
