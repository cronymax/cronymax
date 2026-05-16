// rebuild_trace --- regenerate runs/<run_id>/trace.jsonl from the SQLite
// events table. Useful when the JSONL sidecar has been deleted, truncated,
// or fallen out of sync with the canonical event store.
//
// Usage:
//   rebuild_trace <space_root> <flow_id> <run_id>
//
// `space_root` is the workspace directory (the one containing `.cronymax/`).
// The output is written atomically (.tmp + rename) to:
//   <space_root>/.cronymax/flows/<flow_id>/runs/<run_id>/trace.jsonl
//
// All events for the given run are included, ordered by id (UUIDv7 sortable
// = chronological).

#include <algorithm>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

#include "event_bus/app_event.h"
#include "event_bus/event_bus.h"
#include "runtime/space_store.h"

int main(int argc, char** argv) {
  if (argc != 4) {
    std::fprintf(stderr,
                 "usage: rebuild_trace <space_root> <flow_id> <run_id>\n");
    return 2;
  }
  const std::filesystem::path root = argv[1];
  const std::string flow_id = argv[2];
  const std::string run_id = argv[3];

  cronymax::SpaceStore store;
  if (!store.Open(root / ".cronymax" / "space.db")) {
    std::fprintf(stderr, "rebuild_trace: open SpaceStore failed\n");
    return 1;
  }
  cronymax::event_bus::EventBus bus(&store, std::string(), root);

  // Page through with descending order, then reverse to get oldest-first.
  cronymax::event_bus::ListQuery q;
  q.scope.flow_id = flow_id;
  q.scope.run_id = run_id;
  q.limit = 1000;
  std::vector<cronymax::event_bus::AppEvent> all;
  while (true) {
    auto res = bus.List(q);
    for (auto& e : res.events)
      all.push_back(std::move(e));
    if (res.cursor.empty())
      break;
    q.before_id = res.cursor;
  }
  std::reverse(all.begin(), all.end());

  const auto out_dir = root / ".cronymax" / "flows" / flow_id / "runs" / run_id;
  std::error_code ec;
  std::filesystem::create_directories(out_dir, ec);
  if (ec) {
    std::fprintf(stderr, "rebuild_trace: mkdir failed: %s\n",
                 ec.message().c_str());
    return 1;
  }
  const auto out_path = out_dir / "trace.jsonl";
  const auto tmp_path = out_dir / "trace.jsonl.tmp";
  {
    std::ofstream out(tmp_path, std::ios::binary | std::ios::trunc);
    if (!out) {
      std::fprintf(stderr, "rebuild_trace: open tmp failed\n");
      return 1;
    }
    for (const auto& e : all)
      out << e.ToJson() << "\n";
  }
  std::filesystem::rename(tmp_path, out_path, ec);
  if (ec) {
    std::fprintf(stderr, "rebuild_trace: rename failed: %s\n",
                 ec.message().c_str());
    return 1;
  }
  std::printf("rebuild_trace: wrote %zu events to %s\n", all.size(),
              out_path.string().c_str());
  return 0;
}
