// app/browser/shells/bridge_space.cc
// space.* channels, WireSpaceEventCallback, ResubscribeSpace, OnSpaceSwitch.

#include "browser/bridge_handler.h"

#include <nlohmann/json.hpp>

#include "event_bus/app_event.h"
#include "event_bus/event_bus.h"
#include "include/base/cef_callback.h"
#include "include/cef_task.h"
#include "include/wrapper/cef_closure_task.h"

namespace cronymax {
namespace {

nlohmann::json SpaceToJson(const Space& sp) {
  return nlohmann::json{
      {"id", sp.id},
      {"name", sp.name},
      {"root_path", sp.workspace_root.string()},
      {"profile_id", sp.profile_id},
  };
}

}  // namespace

// ---------------------------------------------------------------------------
// RegisterSpaceHandlers — install browser.space.* in the BridgeRegistry.
// ---------------------------------------------------------------------------

void RegisterSpaceHandlers(BridgeRegistry& r, BridgeHandler* h) {
  r.add("browser.space.list", [h](BridgeCtx ctx) {
    nlohmann::json arr = nlohmann::json::array();
    const auto* active_sp = h->space_manager_->ActiveSpace();
    for (const auto& sp : h->space_manager_->spaces()) {
      arr.push_back({
          {"id", sp->id},
          {"name", sp->name},
          {"root_path", sp->workspace_root.string()},
          {"active", active_sp && sp->id == active_sp->id},
      });
    }
    ctx.callback->Success(arr);
  });

  r.add("browser.space.create", [h](BridgeCtx ctx) {
    const auto& j = ctx.payload;
    const std::string root =
        j.is_object() ? j.value("root_path", std::string{}) : std::string{};
    const std::string profile_id =
        j.is_object() ? j.value("profile_id", std::string{"default"})
                      : std::string{"default"};
    if (root.empty()) {
      ctx.callback->Failure(400, "root_path required");
      return;
    }
    const auto id =
        h->space_manager_->CreateSpace(std::filesystem::path(root), profile_id);
    if (id.empty()) {
      ctx.callback->Failure(500, "failed to create space (path may not exist)");
      return;
    }
    for (const auto& s : h->space_manager_->spaces()) {
      if (s->id == id) {
        const auto sj = SpaceToJson(*s);
        ctx.callback->Success(sj);
        h->SendBrowserEvent(ctx.browser, "space.created", sj.dump());
        return;
      }
    }
    ctx.callback->Success(nlohmann::json{{"id", id}});
  });

  r.add("browser.space.switch", [h](BridgeCtx ctx) {
    const auto& j = ctx.payload;
    const std::string id =
        j.is_object() ? j.value("space_id", std::string{}) : std::string{};
    if (!h->space_manager_->SwitchTo(id)) {
      ctx.callback->Failure(404, "space not found");
      return;
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  r.add("browser.space.delete", [h](BridgeCtx ctx) {
    const auto& j = ctx.payload;
    const std::string id =
        j.is_object() ? j.value("space_id", std::string{}) : std::string{};
    if (!h->space_manager_->DeleteSpace(id)) {
      ctx.callback->Failure(404, "space not found");
      return;
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
    h->SendBrowserEvent(ctx.browser, "space.deleted",
                        nlohmann::json{{"space_id", id}}.dump());
  });
}

// ---------------------------------------------------------------------------
// WireSpaceEventCallback
// ---------------------------------------------------------------------------

int64_t BridgeHandler::WireSpaceEventCallback(const std::string& space_id) {
  return runtime_proxy_->SubscribeEvents(
      [this, space_id](const nlohmann::json& event) {
        if (shell_cbs_.broadcast_event)
          shell_cbs_.broadcast_event("event", event.dump());

        // For file_edited, git_commit_created, git_pushed: also write to the
        // AppEvent bus so events.list/subscribe picks them up in the channel
        // panel.
        if (!event.contains("event") || !event["event"].is_object())
          return;
        const auto& inner = event["event"];
        if (!inner.contains("payload") || !inner["payload"].is_object())
          return;
        const auto& pl = inner["payload"];
        const std::string kind_str = pl.value("kind", std::string{});

        event_bus::AppEventKind target_kind;
        bool is_target = false;
        if (kind_str == "file_edited") {
          target_kind = event_bus::AppEventKind::kFileEdited;
          is_target = true;
        } else if (kind_str == "git_commit_created") {
          target_kind = event_bus::AppEventKind::kGitCommitCreated;
          is_target = true;
        } else if (kind_str == "git_pushed") {
          target_kind = event_bus::AppEventKind::kGitPushed;
          is_target = true;
        }

        if (!is_target)
          return;

        Space* sp = nullptr;
        for (const auto& s : space_manager_->spaces()) {
          if (s->id == space_id) {
            sp = s.get();
            break;
          }
        }
        if (!sp || !sp->event_bus)
          return;

        event_bus::AppEvent evt;
        evt.kind = target_kind;
        evt.space_id = space_id;
        evt.run_id = pl.value("run_id", std::string{});
        evt.session_id = pl.value("session_id", std::string{});

        nlohmann::json payload_obj = nlohmann::json::object();
        if (kind_str == "file_edited") {
          payload_obj["path"] = pl.value("path", std::string{});
          payload_obj["diff"] = pl.value("diff", std::string{});
          if (!evt.session_id.empty())
            payload_obj["session_id"] = evt.session_id;
        } else if (kind_str == "git_commit_created") {
          payload_obj["hash"] = pl.value("hash", std::string{});
          payload_obj["message"] = pl.value("message", std::string{});
          payload_obj["files_changed"] = pl.contains("files_changed")
                                             ? pl["files_changed"]
                                             : nlohmann::json::array();
          if (!evt.session_id.empty())
            payload_obj["session_id"] = evt.session_id;
        } else if (kind_str == "git_pushed") {
          payload_obj["remote"] = pl.value("remote", std::string{});
          payload_obj["branch"] = pl.value("branch", std::string{});
          payload_obj["commits_pushed"] = pl.value("commits_pushed", 0);
          if (!evt.session_id.empty())
            payload_obj["session_id"] = evt.session_id;
        }
        evt.payload = std::move(payload_obj);
        sp->event_bus->Append(std::move(evt));
      });
}

// ---------------------------------------------------------------------------
// ResubscribeSpace
// ---------------------------------------------------------------------------

void BridgeHandler::ResubscribeSpace(
    const std::string& space_id,
    uint64_t gen,
    int64_t delay_ms,
    std::shared_ptr<std::atomic<int>> pending) {
  if (restart_generation_.load(std::memory_order_relaxed) != gen)
    return;
  if (!runtime_proxy_)
    return;

  if (delay_ms > 0) {
    CefPostDelayedTask(
        TID_UI,
        base::BindOnce(&BridgeHandler::ResubscribeSpace, base::Unretained(this),
                       space_id, gen, delay_ms, pending),
        delay_ms);
    return;
  }

  nlohmann::json req = {{"kind", "subscribe"},
                        {"topic", "space/" + space_id + "/events"}};
  runtime_proxy_->SendControl(std::move(req), [this, space_id, gen, delay_ms,
                                               pending](nlohmann::json resp,
                                                        bool is_error) {
    if (restart_generation_.load(std::memory_order_relaxed) != gen)
      return;
    if (is_error) {
      int64_t next_delay =
          delay_ms == 0 ? 100
                        : std::min(delay_ms * 2, static_cast<int64_t>(30'000));
      CefPostDelayedTask(TID_UI,
                         base::BindOnce(&BridgeHandler::ResubscribeSpace,
                                        base::Unretained(this), space_id, gen,
                                        next_delay, pending),
                         next_delay);
      return;
    }

    SpaceRuntimeSub sub;
    sub.runtime_sub_id = resp.value("subscription", std::string{});
    sub.ev_token = WireSpaceEventCallback(space_id);
    {
      std::lock_guard<std::mutex> g(space_subs_mu_);
      space_runtime_subs_[space_id] = std::move(sub);
    }

    if (--(*pending) == 0) {
      if (shell_cbs_.broadcast_event)
        shell_cbs_.broadcast_event("runtime.reconnected", "{}");
    }
  });
}

// ---------------------------------------------------------------------------
// OnSpaceSwitch
// ---------------------------------------------------------------------------

void BridgeHandler::OnSpaceSwitch(const std::string& old_space_id,
                                  const std::string& new_space_id) {
  if (!runtime_proxy_)
    return;

  if (!old_space_id.empty()) {
    SpaceRuntimeSub old_sub;
    {
      std::lock_guard<std::mutex> g(space_subs_mu_);
      auto it = space_runtime_subs_.find(old_space_id);
      if (it != space_runtime_subs_.end()) {
        old_sub = it->second;
        space_runtime_subs_.erase(it);
      }
    }
    if (old_sub.ev_token >= 0)
      runtime_proxy_->UnsubscribeEvents(old_sub.ev_token);
    if (!old_sub.runtime_sub_id.empty()) {
      nlohmann::json req = {
          {"kind", "unsubscribe"},
          {"subscription", old_sub.runtime_sub_id},
      };
      runtime_proxy_->SendControl(std::move(req), [](nlohmann::json, bool) {});
    }
  }

  if (!new_space_id.empty()) {
    nlohmann::json req = {
        {"kind", "subscribe"},
        {"topic", "space/" + new_space_id + "/events"},
    };
    runtime_proxy_->SendControl(
        std::move(req),
        [this, new_space_id](nlohmann::json resp, bool is_error) {
          if (is_error)
            return;
          SpaceRuntimeSub sub;
          sub.runtime_sub_id = resp.value("subscription", std::string{});
          sub.ev_token = WireSpaceEventCallback(new_space_id);
          std::lock_guard<std::mutex> g(space_subs_mu_);
          space_runtime_subs_[new_space_id] = std::move(sub);
        });
  }
}

}  // namespace cronymax
