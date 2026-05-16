// app/browser/shells/bridge_workspace.cc
// workspace.* and llm.config / llm.providers channels.

#include "browser/bridge_handler.h"

#include <fstream>
#include <sstream>

#include <nlohmann/json.hpp>

namespace cronymax {

// ---------------------------------------------------------------------------
// RegisterWorkspaceHandlers — install browser.llm.* and browser.workspace.*
// channels in the BridgeRegistry.
// ---------------------------------------------------------------------------

void RegisterWorkspaceHandlers(BridgeRegistry& r, BridgeHandler* h) {
  // ── llm.config ────────────────────────────────────────────────────────────

  r.add("browser.llm.config.get", [h](BridgeCtx ctx) {
    const auto cfg = h->space_manager_->store().GetLlmConfig();
    ctx.callback->Success(
        nlohmann::json{{"base_url", cfg.base_url}, {"api_key", cfg.api_key}});
  });

  r.add("browser.llm.config.set", [h](BridgeCtx ctx) {
    const auto& j = ctx.payload;
    LlmConfig cfg;
    cfg.base_url =
        j.is_object() ? j.value("base_url", std::string{}) : std::string{};
    cfg.api_key =
        j.is_object() ? j.value("api_key", std::string{}) : std::string{};
    h->space_manager_->store().SetLlmConfig(cfg);
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── llm.providers ─────────────────────────────────────────────────────────

  r.add("browser.llm.providers.get", [h](BridgeCtx ctx) {
    const std::string raw = h->space_manager_->store().GetKv("llm.providers");
    const std::string active =
        h->space_manager_->store().GetKv("llm.active_provider_id");
    ctx.callback->Success(nlohmann::json{{"raw", raw}, {"active_id", active}});
  });

  r.add("browser.llm.providers.set", [h](BridgeCtx ctx) {
    const auto& j = ctx.payload;
    const std::string raw =
        j.is_object() ? j.value("raw", std::string{}) : std::string{};
    const std::string active =
        j.is_object() ? j.value("active_id", std::string{}) : std::string{};
    h->space_manager_->store().SetKv("llm.providers", raw);
    h->space_manager_->store().SetKv("llm.active_provider_id", active);
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── workspace ─────────────────────────────────────────────────────────────

  r.add("browser.workspace.gitignore_suggestions", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    static const std::vector<std::string> kSuggested = {
        ".cronymax/flows/*/runs/*/trace.jsonl",
        ".cronymax/flows/*/runs/*/reviews.json",
    };
    const auto gitignore_path = sp->workspace_root / ".gitignore";
    std::string gitignore_content;
    {
      std::ifstream in(gitignore_path);
      if (in) {
        std::ostringstream ss;
        ss << in.rdbuf();
        gitignore_content = ss.str();
      }
    }
    nlohmann::json arr = nlohmann::json::array();
    for (const auto& entry : kSuggested) {
      if (gitignore_content.find(entry) == std::string::npos)
        arr.push_back(entry);
    }
    ctx.callback->Success(nlohmann::json{{"missing", arr}});
  });

  r.add("browser.workspace.prompts.list", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    const auto prompts_dir = sp->workspace_root / ".cronymax" / "prompts";
    nlohmann::json arr = nlohmann::json::array();
    std::error_code ec;
    for (const auto& entry :
         std::filesystem::directory_iterator(prompts_dir, ec)) {
      const auto& p = entry.path();
      if (p.extension() == ".md" && p.stem().extension() == ".prompt") {
        std::ifstream f(p);
        if (f) {
          std::ostringstream ss;
          ss << f.rdbuf();
          arr.push_back(
              {{"name", p.stem().stem().string()}, {"content", ss.str()}});
        }
      }
    }
    ctx.callback->Success(nlohmann::json{{"prompts", arr}});
  });

  r.add("browser.workspace.prompt.save", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    const auto& jp = ctx.payload;
    if (!jp.is_object()) {
      ctx.callback->Failure(400, "invalid payload");
      return;
    }
    std::string name, content;
    {
      auto it = jp.find("name");
      if (it == jp.end() || !it->is_string()) {
        ctx.callback->Failure(400, "name required");
        return;
      }
      name = it->get<std::string>();
    }
    {
      auto it = jp.find("content");
      if (it == jp.end() || !it->is_string()) {
        ctx.callback->Failure(400, "content required");
        return;
      }
      content = it->get<std::string>();
    }
    // Reject path-traversal characters.
    if (name.empty() || name.find('/') != std::string::npos ||
        name.find('\\') != std::string::npos ||
        name.find("..") != std::string::npos ||
        name.find('\0') != std::string::npos) {
      ctx.callback->Success(
          nlohmann::json{{"ok", false}, {"error", "invalid name"}});
      return;
    }
    const auto prompts_dir = sp->workspace_root / ".cronymax" / "prompts";
    std::error_code ec;
    std::filesystem::create_directories(prompts_dir, ec);
    if (ec) {
      ctx.callback->Success(
          nlohmann::json{{"ok", false}, {"error", ec.message()}});
      return;
    }
    const auto target = prompts_dir / (name + ".prompt.md");
    std::ofstream f(target, std::ios::out | std::ios::trunc);
    if (!f) {
      ctx.callback->Success(nlohmann::json{
          {"ok", false}, {"error", "failed to open file for writing"}});
      return;
    }
    f << content;
    f.close();
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });
}

}  // namespace cronymax
