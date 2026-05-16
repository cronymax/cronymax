#include <filesystem>
#include <iostream>
#include <sstream>
#include <string>

#include "runtime/space_manager.h"
#include "runtime/space_store.h"

namespace {

std::string JoinArgs(int argc, char** argv, int start) {
  std::ostringstream out;
  for (int i = start; i < argc; ++i) {
    if (i > start) {
      out << ' ';
    }
    out << argv[i];
  }
  return out.str();
}

void Usage() {
  std::cerr << "Usage:\n"
            << "  native_probe read <workspace> <relative-path>\n"
            << "  native_probe agent <workspace> <task...>\n"
            << "  native_probe space-store <db-path>\n"
            << "  native_probe space-manager <db-path> <workspace>\n"
            << "  native_probe file-boundary <workspace>\n";
}

// ---------------------------------------------------------------------------
// 9.3: SpaceStore smoke test — open DB, create / list / update / delete a
// Space and a tab, persist a terminal block, write & read an LLM config.
// Prints PASS/FAIL lines so the output is greppable in CI.
// ---------------------------------------------------------------------------

int RunSpaceStore(const std::filesystem::path& db_path) {
  cronymax::SpaceStore store;
  if (!store.Open(db_path)) {
    std::cerr << "FAIL: SpaceStore::Open(" << db_path << ")\n";
    return 1;
  }
  std::cout << "PASS: SpaceStore::Open\n";

  cronymax::SpaceRow s;
  s.id = "probe-space-1";
  s.name = "Probe";
  s.root_path = std::filesystem::current_path().string();
  s.created_at = 1;
  s.last_active = 1;
  if (!store.CreateSpace(s)) {
    std::cerr << "FAIL: CreateSpace\n";
    return 1;
  }
  std::cout << "PASS: CreateSpace\n";

  bool found = false;
  for (const auto& row : store.ListSpaces()) {
    if (row.id == s.id) {
      found = true;
      break;
    }
  }
  if (!found) {
    std::cerr << "FAIL: ListSpaces missing probe row\n";
    return 1;
  }
  std::cout << "PASS: ListSpaces\n";

  cronymax::BrowserTabRow t;
  t.space_id = s.id;
  t.url = "https://example.com";
  t.title = "Example";
  const int64_t tab_id = store.CreateTab(t);
  if (tab_id <= 0) {
    std::cerr << "FAIL: CreateTab\n";
    return 1;
  }
  std::cout << "PASS: CreateTab id=" << tab_id << "\n";

  t.id = tab_id;
  t.title = "Example (updated)";
  if (!store.UpdateTab(t)) {
    std::cerr << "FAIL: UpdateTab\n";
    return 1;
  }
  std::cout << "PASS: UpdateTab\n";

  if (store.ListTabsForSpace(s.id).empty()) {
    std::cerr << "FAIL: ListTabsForSpace empty\n";
    return 1;
  }
  std::cout << "PASS: ListTabsForSpace\n";

  cronymax::LlmConfig cfg;
  cfg.base_url = "http://localhost:11434/v1";
  cfg.api_key = "probe-key";
  if (!store.SetLlmConfig(cfg)) {
    std::cerr << "FAIL: SetLlmConfig\n";
    return 1;
  }
  const auto got = store.GetLlmConfig();
  if (got.base_url != cfg.base_url || got.api_key != cfg.api_key) {
    std::cerr << "FAIL: GetLlmConfig roundtrip\n";
    return 1;
  }
  std::cout << "PASS: LlmConfig roundtrip\n";

  store.DeleteTab(tab_id);
  if (!store.DeleteSpace(s.id)) {
    std::cerr << "FAIL: DeleteSpace\n";
    return 1;
  }
  std::cout << "PASS: DeleteSpace\n";
  return 0;
}

// ---------------------------------------------------------------------------
// 9.3: SpaceManager smoke test — Init, CreateSpace, switch, callback fires,
// Active resolves, DeleteSpace cleans up.
// ---------------------------------------------------------------------------

int RunSpaceManager(const std::filesystem::path& db_path,
                    const std::filesystem::path& workspace) {
  cronymax::SpaceManager mgr;
  if (!mgr.Init(db_path)) {
    std::cerr << "FAIL: SpaceManager::Init\n";
    return 1;
  }
  std::cout << "PASS: SpaceManager::Init\n";

  std::string switched_to;
  mgr.SetSwitchCallback(
      [&](const std::string& /*old_id*/, const std::string& new_id) {
        switched_to = new_id;
      });

  const std::string a = mgr.CreateSpace("probe-A", workspace);
  const std::string b = mgr.CreateSpace("probe-B", workspace);
  if (a.empty() || b.empty()) {
    std::cerr << "FAIL: CreateSpace returned empty id\n";
    return 1;
  }
  std::cout << "PASS: CreateSpace x2\n";

  if (!mgr.SwitchTo(a)) {
    std::cerr << "FAIL: SwitchTo(a)\n";
    return 1;
  }
  if (switched_to != a) {
    std::cerr << "FAIL: switch callback expected " << a << " got "
              << switched_to << "\n";
    return 1;
  }
  if (!mgr.ActiveSpace() || mgr.ActiveSpace()->id != a) {
    std::cerr << "FAIL: ActiveSpace mismatch after SwitchTo(a)\n";
    return 1;
  }
  std::cout << "PASS: SwitchTo + ActiveSpace + callback\n";

  if (!mgr.DeleteSpace(a)) {
    std::cerr << "FAIL: DeleteSpace(a)\n";
    return 1;
  }
  if (!mgr.DeleteSpace(b)) {
    std::cerr << "FAIL: DeleteSpace(b)\n";
    return 1;
  }
  std::cout << "PASS: DeleteSpace x2\n";
  return 0;
}

// ---------------------------------------------------------------------------
// 9.4: file boundary test removed — AgentRuntime deleted (Phase 1 migration).
// The filesystem capability is now enforced by the Rust runtime directly.
// ---------------------------------------------------------------------------

}  // namespace

int main(int argc, char** argv) {
  if (argc < 3) {
    Usage();
    return 2;
  }

  const std::string mode = argv[1];

  if (mode == "space-store") {
    return RunSpaceStore(argv[2]);
  }

  if (mode == "space-manager") {
    if (argc < 4) {
      Usage();
      return 2;
    }
    return RunSpaceManager(argv[2], argv[3]);
  }

  const std::filesystem::path workspace = argv[2];

  if (mode == "file-boundary") {
    std::cerr << "file-boundary mode removed (AgentRuntime deleted in Phase 1 "
                 "migration)\n";
    return 1;
  }

  if (mode == "read") {
    if (argc < 4) {
      Usage();
      return 2;
    }
    cronymax::FileBroker broker(workspace);
    const auto result =
        broker.ReadText(cronymax::Actor::kAgent, workspace / argv[3]);
    if (!result.ok) {
      std::cerr << result.error << "\n";
      return 1;
    }
    std::cout << result.data;
    return 0;
  }

  if (mode == "agent") {
    std::cerr
        << "agent mode removed (AgentRuntime deleted in Phase 1 migration)\n";
    return 1;
  }

  Usage();
  return 2;
}
