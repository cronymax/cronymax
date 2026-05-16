// app/browser/shells/bridge_events.cc
// Event channels: activity.snapshot, events.*, inbox.*, notifications.*,
// profiles.*

#include "browser/bridge_handler.h"

#include <nlohmann/json.hpp>

#include "event_bus/app_event.h"
#include "event_bus/event_bus.h"
#include "include/wrapper/cef_helpers.h"

namespace cronymax {
namespace {

std::string AppEventToJson(const event_bus::AppEvent& e) {
  return e.ToJson();
}

std::string InboxRowToJson(const event_bus::InboxRow& r) {
  nlohmann::json j = {
      {"event_id", r.event_id},
      {"state", event_bus::InboxStateToString(r.state)},
      {"flow_id", r.flow_id},
      {"kind", r.kind},
  };
  if (r.snooze_until.has_value())
    j["snooze_until"] = *r.snooze_until;
  return j.dump();
}

nlohmann::json ProfileRecordToJson(const ProfileRecord& r) {
  auto to_arr = [](const std::vector<std::string>& v) {
    auto arr = nlohmann::json::array();
    for (const auto& s : v)
      arr.push_back(s);
    return arr;
  };
  return nlohmann::json{
      {"id", r.id},
      {"name", r.name},
      {"memory_id", r.memory_id},
      {"allow_network", r.allow_network},
      {"extra_read_paths", to_arr(r.extra_read_paths)},
      {"extra_write_paths", to_arr(r.extra_write_paths)},
      {"extra_deny_paths", to_arr(r.extra_deny_paths)},
  };
}

ProfileRules PayloadToProfileRules(const nlohmann::json& jp) {
  ProfileRules rules;
  rules.name = jp.value("name", std::string{});
  rules.memory_id = jp.value("memory_id", std::string{});
  rules.allow_network = jp.value("allow_network", true);
  if (jp.contains("extra_read_paths") && jp["extra_read_paths"].is_array())
    for (const auto& p : jp["extra_read_paths"])
      if (p.is_string())
        rules.extra_read_paths.push_back(p);
  if (jp.contains("extra_write_paths") && jp["extra_write_paths"].is_array())
    for (const auto& p : jp["extra_write_paths"])
      if (p.is_string())
        rules.extra_write_paths.push_back(p);
  if (jp.contains("extra_deny_paths") && jp["extra_deny_paths"].is_array())
    for (const auto& p : jp["extra_deny_paths"])
      if (p.is_string())
        rules.extra_deny_paths.push_back(p);
  return rules;
}

}  // namespace

// ---------------------------------------------------------------------------
// RegisterEventsHandlers — install browser.activity/events/inbox/
//   notifications/profiles channels in the BridgeRegistry.
// ---------------------------------------------------------------------------

void RegisterEventsHandlers(BridgeRegistry& r, BridgeHandler* h) {
  // ── activity.snapshot ────────────────────────────────────────────────────
  r.add("browser.activity.snapshot", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    if (!h->runtime_proxy_) {
      ctx.callback->Failure(503, "runtime not connected");
      return;
    }
    nlohmann::json req = {{"kind", "get_space_snapshot"}, {"space_id", sp->id}};
    h->runtime_proxy_->SendControl(std::move(req), [cb = ctx.callback](
                                                       nlohmann::json resp,
                                                       bool is_error) {
      if (is_error) {
        cb->Failure(500, resp.value("error", nlohmann::json{})
                             .value("message", "get_space_snapshot failed"));
        return;
      }
      cb->Success(resp);
    });
  });

  // ── events.list ──────────────────────────────────────────────────────────
  r.add("browser.events.list", [h](BridgeCtx ctx) {
    CEF_REQUIRE_UI_THREAD();
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp || !sp->event_bus) {
      ctx.callback->Failure(503, "event bus not ready");
      return;
    }
    auto* bus = sp->event_bus.get();
    const auto& payload = ctx.payload;
    event_bus::ListQuery q;
    q.scope.flow_id = payload.value("flow_id", std::string{});
    q.scope.run_id = payload.value("run_id", std::string{});
    q.before_id = payload.value("before_id", std::string{});
    long long lim = payload.is_object() && payload.contains("limit") &&
                            payload["limit"].is_number_integer()
                        ? payload["limit"].get<long long>()
                        : 0LL;
    if (lim > 0)
      q.limit = static_cast<int>(lim);
    auto res = bus->List(q);
    nlohmann::json events_arr = nlohmann::json::array();
    for (const auto& e : res.events)
      events_arr.push_back(
          nlohmann::json::parse(AppEventToJson(e), nullptr, false));
    ctx.callback->Success(
        nlohmann::json{{"events", events_arr}, {"cursor", res.cursor}});
  });

  // ── events.subscribe ─────────────────────────────────────────────────────
  r.add("browser.events.subscribe", [h](BridgeCtx ctx) {
    CEF_REQUIRE_UI_THREAD();
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp || !sp->event_bus) {
      ctx.callback->Failure(503, "event bus not ready");
      return;
    }
    auto* bus = sp->event_bus.get();
    const auto& payload = ctx.payload;
    event_bus::Scope scope;
    scope.flow_id = payload.value("flow_id", std::string{});
    scope.run_id = payload.value("run_id", std::string{});
    auto cbs = h->shell_cbs_;
    auto token = bus->Subscribe(scope, [cbs](const event_bus::AppEvent& e) {
      if (cbs.broadcast_event)
        cbs.broadcast_event("event", e.ToJson());
    });
    const int bid = ctx.browser ? ctx.browser->GetIdentifier() : 0;
    {
      std::lock_guard<std::mutex> g(h->browser_subs_mutex_);
      h->browser_subs_[bid].push_back(
          [bus, token]() { bus->Unsubscribe(token); });
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── events.append ────────────────────────────────────────────────────────
  r.add("browser.events.append", [h](BridgeCtx ctx) {
    CEF_REQUIRE_UI_THREAD();
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp || !sp->event_bus) {
      ctx.callback->Failure(503, "event bus not ready");
      return;
    }
    auto* bus = sp->event_bus.get();
    const auto& payload = ctx.payload;
    auto kind_str = payload.value("kind", std::string{});
    if (kind_str != "text") {
      ctx.callback->Failure(400, "events.append only accepts kind=text");
      return;
    }
    event_bus::AppEvent evt;
    evt.kind = event_bus::AppEventKind::kText;
    evt.space_id = sp->id;
    evt.flow_id = payload.value("flow_id", std::string{});
    evt.run_id = payload.value("run_id", std::string{});
    evt.agent_id = payload.value("agent_id", std::string{});
    bool have_payload = false;
    if (payload.is_object() && payload.contains("payload") &&
        payload["payload"].is_object()) {
      evt.payload = payload["payload"];
      have_payload = true;
    }
    if (!have_payload) {
      auto body = payload.value("body", std::string{});
      evt.payload = {{"body", body}, {"mentions", nlohmann::json::array()}};
    }
    auto id = bus->Append(std::move(evt));
    ctx.callback->Success(nlohmann::json{{"id", id}});
  });

  // ── inbox.list ───────────────────────────────────────────────────────────
  r.add("browser.inbox.list", [h](BridgeCtx ctx) {
    CEF_REQUIRE_UI_THREAD();
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp || !sp->event_bus) {
      ctx.callback->Failure(503, "event bus not ready");
      return;
    }
    auto* bus = sp->event_bus.get();
    const auto& payload = ctx.payload;
    event_bus::InboxQuery q;
    q.flow_id = payload.value("flow_id", std::string{});
    auto state_str = payload.value("state", std::string{});
    if (!state_str.empty()) {
      event_bus::InboxState s;
      if (event_bus::InboxStateFromString(state_str, &s))
        q.state = s;
    }
    long long lim = payload.is_object() && payload.contains("limit") &&
                            payload["limit"].is_number_integer()
                        ? payload["limit"].get<long long>()
                        : 0LL;
    if (lim > 0)
      q.limit = static_cast<int>(lim);
    auto res = bus->ListInbox(q);
    nlohmann::json rows_arr = nlohmann::json::array();
    for (const auto& r : res.rows)
      rows_arr.push_back(
          nlohmann::json::parse(InboxRowToJson(r), nullptr, false));
    ctx.callback->Success(nlohmann::json{
        {"rows", rows_arr},
        {"unread_count", res.unread_count},
        {"needs_action_count", res.needs_action_count},
    });
  });

  // ── inbox.read ───────────────────────────────────────────────────────────
  r.add("browser.inbox.read", [h](BridgeCtx ctx) {
    CEF_REQUIRE_UI_THREAD();
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp || !sp->event_bus) {
      ctx.callback->Failure(503, "event bus not ready");
      return;
    }
    auto event_id = ctx.payload.value("event_id", std::string{});
    if (event_id.empty()) {
      ctx.callback->Failure(400, "event_id required");
      return;
    }
    if (!sp->event_bus->SetInboxState(event_id, event_bus::InboxState::kRead,
                                      std::nullopt)) {
      ctx.callback->Failure(404, "inbox row not found");
      return;
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── inbox.unread ─────────────────────────────────────────────────────────
  r.add("browser.inbox.unread", [h](BridgeCtx ctx) {
    CEF_REQUIRE_UI_THREAD();
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp || !sp->event_bus) {
      ctx.callback->Failure(503, "event bus not ready");
      return;
    }
    auto event_id = ctx.payload.value("event_id", std::string{});
    if (event_id.empty()) {
      ctx.callback->Failure(400, "event_id required");
      return;
    }
    if (!sp->event_bus->SetInboxState(event_id, event_bus::InboxState::kUnread,
                                      std::nullopt)) {
      ctx.callback->Failure(404, "inbox row not found");
      return;
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── inbox.snooze ─────────────────────────────────────────────────────────
  r.add("browser.inbox.snooze", [h](BridgeCtx ctx) {
    CEF_REQUIRE_UI_THREAD();
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp || !sp->event_bus) {
      ctx.callback->Failure(503, "event bus not ready");
      return;
    }
    const auto& payload = ctx.payload;
    auto event_id = payload.value("event_id", std::string{});
    if (event_id.empty()) {
      ctx.callback->Failure(400, "event_id required");
      return;
    }
    long long until = payload.is_object() && payload.contains("snooze_until") &&
                              payload["snooze_until"].is_number_integer()
                          ? payload["snooze_until"].get<long long>()
                          : 0LL;
    if (until <= 0) {
      ctx.callback->Failure(400, "snooze_until required for inbox.snooze");
      return;
    }
    if (!sp->event_bus->SetInboxState(event_id, event_bus::InboxState::kSnoozed,
                                      std::optional<long long>(until))) {
      ctx.callback->Failure(404, "inbox row not found");
      return;
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── notifications.get_prefs ──────────────────────────────────────────────
  r.add("browser.notifications.get_prefs", [h](BridgeCtx ctx) {
    CEF_REQUIRE_UI_THREAD();
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp || !sp->event_bus) {
      ctx.callback->Failure(503, "event bus not ready");
      return;
    }
    auto kinds = sp->event_bus->ListEnabledNotificationKinds();
    nlohmann::json enabled = nlohmann::json::array();
    for (const auto& k : kinds)
      enabled.push_back(k);
    ctx.callback->Success(nlohmann::json{{"enabled", enabled}});
  });

  // ── notifications.set_kind_pref ──────────────────────────────────────────
  r.add("browser.notifications.set_kind_pref", [h](BridgeCtx ctx) {
    CEF_REQUIRE_UI_THREAD();
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp || !sp->event_bus) {
      ctx.callback->Failure(503, "event bus not ready");
      return;
    }
    auto kind = ctx.payload.value("kind", std::string{});
    if (kind.empty()) {
      ctx.callback->Failure(400, "kind required");
      return;
    }
    bool enabled =
        ctx.payload.is_object() ? ctx.payload.value("enabled", false) : false;
    sp->event_bus->SetKindNotificationEnabled(kind, enabled);
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── profiles.list ────────────────────────────────────────────────────────
  r.add("browser.profiles.list", [h](BridgeCtx ctx) {
    ProfileStore& ps = h->space_manager_->profile_store();
    const auto records = ps.List();
    auto arr = nlohmann::json::array();
    for (const auto& r : records)
      arr.push_back(ProfileRecordToJson(r));
    ctx.callback->Success(arr);
  });

  // ── profiles.create ──────────────────────────────────────────────────────
  r.add("browser.profiles.create", [h](BridgeCtx ctx) {
    const auto& jp = ctx.payload;
    if (!jp.is_object()) {
      ctx.callback->Failure(400, "payload must be an object");
      return;
    }
    ProfileRules rules = PayloadToProfileRules(jp);
    if (rules.name.empty()) {
      ctx.callback->Failure(400, "name required");
      return;
    }
    ProfileStore& ps = h->space_manager_->profile_store();
    std::string new_id;
    const auto err = ps.Create(rules, &new_id);
    if (err == ProfileStoreError::kAlreadyExists) {
      ctx.callback->Failure(409, "profile name already exists");
      return;
    }
    if (err == ProfileStoreError::kIoError) {
      ctx.callback->Failure(500, "I/O error writing profile");
      return;
    }
    if (const auto rec = ps.Get(new_id))
      ctx.callback->Success(ProfileRecordToJson(*rec));
    else
      ctx.callback->Success(nlohmann::json{{"id", new_id}});
  });

  // ── profiles.update ──────────────────────────────────────────────────────
  r.add("browser.profiles.update", [h](BridgeCtx ctx) {
    const auto& jp = ctx.payload;
    if (!jp.is_object()) {
      ctx.callback->Failure(400, "payload must be an object");
      return;
    }
    const std::string id = jp.value("id", std::string{});
    if (id.empty()) {
      ctx.callback->Failure(400, "id required");
      return;
    }
    ProfileRules rules = PayloadToProfileRules(jp);
    if (rules.name.empty()) {
      ctx.callback->Failure(400, "name required");
      return;
    }
    ProfileStore& ps = h->space_manager_->profile_store();
    const auto err = ps.Update(id, rules);
    if (err == ProfileStoreError::kNotFound) {
      ctx.callback->Failure(404, "profile not found");
      return;
    }
    if (err == ProfileStoreError::kIoError) {
      ctx.callback->Failure(500, "I/O error writing profile");
      return;
    }
    if (const auto rec = ps.Get(id))
      ctx.callback->Success(ProfileRecordToJson(*rec));
    else
      ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── profiles.delete ──────────────────────────────────────────────────────
  r.add("browser.profiles.delete", [h](BridgeCtx ctx) {
    const auto& jp = ctx.payload;
    if (!jp.is_object()) {
      ctx.callback->Failure(400, "payload must be an object");
      return;
    }
    const std::string id = jp.value("id", std::string{});
    if (id.empty()) {
      ctx.callback->Failure(400, "id required");
      return;
    }
    std::vector<std::string> space_profile_ids;
    for (const auto& s : h->space_manager_->spaces())
      space_profile_ids.push_back(s->profile_id);
    ProfileStore& ps = h->space_manager_->profile_store();
    const auto err = ps.Delete(id, space_profile_ids);
    if (err == ProfileStoreError::kNotFound) {
      ctx.callback->Failure(404, "profile not found");
      return;
    }
    if (err == ProfileStoreError::kCannotDeleteDefault) {
      ctx.callback->Failure(403, "cannot delete default profile");
      return;
    }
    if (err == ProfileStoreError::kInUse) {
      ctx.callback->Failure(409, "profile is in use by one or more spaces");
      return;
    }
    if (err == ProfileStoreError::kIoError) {
      ctx.callback->Failure(500, "I/O error deleting profile");
      return;
    }
    ctx.callback->Success(nlohmann::json{{"ok", true}});
  });

  // ── profiles.check_paths ─────────────────────────────────────────────────
  r.add("browser.profiles.check_paths", [](BridgeCtx ctx) {
    const auto& jp = ctx.payload;
    if (!jp.is_object() || !jp.contains("paths") || !jp["paths"].is_array()) {
      ctx.callback->Failure(400, "paths array required");
      return;
    }
    nlohmann::json missing = nlohmann::json::array();
    for (const auto& entry : jp["paths"]) {
      if (!entry.is_string())
        continue;
      const std::string p = entry.get<std::string>();
      if (p.empty())
        continue;
      std::error_code ec;
      if (!std::filesystem::exists(std::filesystem::path(p), ec))
        missing.push_back(p);
    }
    ctx.callback->Success(nlohmann::json{{"missing", std::move(missing)}});
  });
}

}  // namespace cronymax
