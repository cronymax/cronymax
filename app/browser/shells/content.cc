// app/browser/shells/bridge_content.cc
// Content channels: mention.*, document.*, review.*

#include "browser/bridge_handler.h"

#include <nlohmann/json.hpp>

namespace cronymax {

// ---------------------------------------------------------------------------
// RegisterContentHandlers — install content channels in the BridgeRegistry.
// ---------------------------------------------------------------------------

void RegisterContentHandlers(BridgeRegistry& r, BridgeHandler* h) {
  // ── mention.user_input ───────────────────────────────────────────────────
  r.add("mention.user_input", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    const auto& jp = ctx.payload;
    auto extract_field = [&](std::string_view key) -> std::string {
      if (!jp.is_object())
        return {};
      auto it = jp.find(std::string(key));
      if (it == jp.end() || !it->is_string())
        return {};
      return it->get<std::string>();
    };
    const auto flow_id = extract_field("flow_id");
    if (flow_id.empty()) {
      ctx.callback->Failure(400, "flow_id required");
      return;
    }
    if (!h->runtime_proxy_) {
      ctx.callback->Failure(503, "runtime not available");
      return;
    }
    const std::string workspace_root = sp->workspace_root.string();
    const std::string text = extract_field("text");
    h->runtime_proxy_->SendControl(
        {
            {"kind", "mention_parse"},
            {"workspace_root", workspace_root},
            {"flow_id", flow_id},
            {"text", text},
        },
        [cb = ctx.callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            cb->Failure(500, resp.value("error", nlohmann::json{})
                                 .value("message", "mention parse error"));
            return;
          }
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          cb->Success(p);
        });
  });

  // ── document.list ────────────────────────────────────────────────────────
  r.add("document.list", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    if (!h->runtime_proxy_) {
      ctx.callback->Failure(503, "runtime not available");
      return;
    }
    const auto& jp = ctx.payload;
    auto extract = [&](std::string_view key) -> std::string {
      if (!jp.is_object())
        return {};
      auto it = jp.find(std::string(key));
      if (it == jp.end() || !it->is_string())
        return {};
      return it->get<std::string>();
    };
    const std::string flow_id = extract("flow");
    if (flow_id.empty()) {
      ctx.callback->Failure(400, "missing 'flow' in payload");
      return;
    }
    const std::string workspace_root = sp->workspace_root.string();
    h->runtime_proxy_->SendControl(
        {
            {"kind", "document_list"},
            {"workspace_root", workspace_root},
            {"flow_id", flow_id},
        },
        [cb = ctx.callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            cb->Failure(500, resp.value("error", nlohmann::json{})
                                 .value("message", "document error"));
            return;
          }
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          cb->Success(p);
        });
  });

  // ── document.read ────────────────────────────────────────────────────────
  r.add("document.read", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    if (!h->runtime_proxy_) {
      ctx.callback->Failure(503, "runtime not available");
      return;
    }
    const auto& jp = ctx.payload;
    auto extract = [&](std::string_view key) -> std::string {
      if (!jp.is_object())
        return {};
      auto it = jp.find(std::string(key));
      if (it == jp.end() || !it->is_string())
        return {};
      return it->get<std::string>();
    };
    const std::string flow_id = extract("flow");
    if (flow_id.empty()) {
      ctx.callback->Failure(400, "missing 'flow' in payload");
      return;
    }
    const std::string workspace_root = sp->workspace_root.string();
    const std::string name = extract("name");
    const std::string rev_str = extract("revision");
    if (name.empty()) {
      ctx.callback->Failure(400, "missing 'name' in payload");
      return;
    }
    nlohmann::json req = {
        {"kind", "document_read"},
        {"workspace_root", workspace_root},
        {"flow_id", flow_id},
        {"name", name},
    };
    if (!rev_str.empty()) {
      if (rev_str.find_first_not_of("0123456789") != std::string::npos) {
        ctx.callback->Failure(400, "bad 'revision' value");
        return;
      }
      req["revision"] = std::atoi(rev_str.c_str());
    }
    h->runtime_proxy_->SendControl(
        std::move(req),
        [cb = ctx.callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            cb->Failure(500, resp.value("error", nlohmann::json{})
                                 .value("message", "document error"));
            return;
          }
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          cb->Success(p);
        });
  });

  // ── document.subscribe ───────────────────────────────────────────────────
  r.add("document.subscribe", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    if (!h->runtime_proxy_) {
      ctx.callback->Failure(503, "runtime not available");
      return;
    }
    const std::string topic = "space/" + sp->id + "/document_events";
    nlohmann::json req_sub = {{"kind", "subscribe"}, {"topic", topic}};
    h->runtime_proxy_->SendControl(
        std::move(req_sub), [h](nlohmann::json resp, bool is_error) {
          if (is_error)
            return;
          h->runtime_proxy_->SubscribeEvents([h](const nlohmann::json& event) {
            if (h->shell_cbs_.broadcast_event)
              h->shell_cbs_.broadcast_event("document.changed", event.dump());
          });
        });
    ctx.callback->Success(
        nlohmann::json{{"ok", true}, {"event", "document.changed"}});
  });

  // ── document.submit ──────────────────────────────────────────────────────
  r.add("document.submit", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    if (!h->runtime_proxy_) {
      ctx.callback->Failure(503, "runtime not available");
      return;
    }
    const auto& jp = ctx.payload;
    auto extract = [&](std::string_view key) -> std::string {
      if (!jp.is_object())
        return {};
      auto it = jp.find(std::string(key));
      if (it == jp.end() || !it->is_string())
        return {};
      return it->get<std::string>();
    };
    const std::string flow_id = extract("flow");
    if (flow_id.empty()) {
      ctx.callback->Failure(400, "missing 'flow' in payload");
      return;
    }
    const std::string workspace_root = sp->workspace_root.string();
    const std::string name = extract("name");
    const std::string content =
        jp.is_object() ? jp.value("content", std::string{}) : std::string{};
    if (name.empty()) {
      ctx.callback->Failure(400, "missing 'name' in payload");
      return;
    }
    h->runtime_proxy_->SendControl(
        {
            {"kind", "document_submit"},
            {"workspace_root", workspace_root},
            {"flow_id", flow_id},
            {"name", name},
            {"content", content},
        },
        [h, flow_id, name, cb = ctx.callback](nlohmann::json resp,
                                              bool is_error) {
          if (is_error) {
            cb->Failure(500, resp.value("error", nlohmann::json{})
                                 .value("message", "submit failed"));
            return;
          }
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          int rev = p.value("revision", 0);
          std::string sha = p.value("sha256", "");
          if (h->shell_cbs_.broadcast_event) {
            h->shell_cbs_.broadcast_event(
                "document.changed",
                nlohmann::json{
                    {"flow", flow_id}, {"name", name}, {"revision", rev}}
                    .dump());
          }
          cb->Success(
              nlohmann::json{{"ok", true}, {"revision", rev}, {"sha", sha}});
        });
  });

  // ── document.suggestion.apply ────────────────────────────────────────────
  r.add("document.suggestion.apply", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    if (!h->runtime_proxy_) {
      ctx.callback->Failure(503, "runtime not available");
      return;
    }
    const auto& jp = ctx.payload;
    auto extract = [&](std::string_view key) -> std::string {
      if (!jp.is_object())
        return {};
      auto it = jp.find(std::string(key));
      if (it == jp.end() || !it->is_string())
        return {};
      return it->get<std::string>();
    };
    const std::string flow_id = extract("flow");
    if (flow_id.empty()) {
      ctx.callback->Failure(400, "missing 'flow' in payload");
      return;
    }
    const std::string workspace_root = sp->workspace_root.string();
    const std::string run_id = extract("run_id");
    const std::string name = extract("name");
    const std::string block_id = extract("block_id");
    const std::string suggestion =
        jp.is_object() ? jp.value("suggestion", std::string{}) : std::string{};
    if (run_id.empty() || name.empty() || block_id.empty() ||
        suggestion.empty()) {
      ctx.callback->Failure(
          400, "missing 'run_id', 'name', 'block_id', or 'suggestion'");
      return;
    }
    h->runtime_proxy_->SendControl(
        {
            {"kind", "document_suggestion_apply"},
            {"workspace_root", workspace_root},
            {"flow_id", flow_id},
            {"run_id", run_id},
            {"name", name},
            {"block_id", block_id},
            {"suggestion", suggestion},
        },
        [h, flow_id, name, cb = ctx.callback](nlohmann::json resp,
                                              bool is_error) {
          if (is_error) {
            cb->Failure(500, resp.value("error", nlohmann::json{})
                                 .value("message", "suggestion_apply failed"));
            return;
          }
          const auto& p = resp.contains("payload") ? resp["payload"] : resp;
          int rev = p.value("new_revision", 0);
          std::string sha = p.value("sha", "");
          if (h->shell_cbs_.broadcast_event) {
            h->shell_cbs_.broadcast_event(
                "document.changed",
                nlohmann::json{
                    {"flow", flow_id}, {"name", name}, {"revision", rev}}
                    .dump());
          }
          cb->Success(nlohmann::json{
              {"ok", true}, {"new_revision", rev}, {"sha", sha}});
        });
  });

  // ── review.list ──────────────────────────────────────────────────────────
  r.add("review.list", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    if (!h->runtime_proxy_) {
      ctx.callback->Failure(503, "runtime not connected");
      return;
    }
    const auto& jp = ctx.payload;
    auto extract = [&](std::string_view key) -> std::string {
      if (!jp.is_object())
        return {};
      auto it = jp.find(std::string(key));
      if (it == jp.end() || !it->is_string())
        return {};
      return it->get<std::string>();
    };
    const std::string run_id = extract("run_id");
    if (run_id.empty()) {
      ctx.callback->Failure(400, "missing 'run_id' in payload");
      return;
    }
    nlohmann::json req = {{"kind", "list_reviews"}, {"run_id", run_id}};
    h->runtime_proxy_->SendControl(
        std::move(req),
        [cb = ctx.callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            cb->Failure(500, resp.value("error", nlohmann::json{})
                                 .value("message", "list_reviews failed"));
            return;
          }
          cb->Success(resp);
        });
  });

  // ── review.approve ───────────────────────────────────────────────────────
  r.add("review.approve", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    if (!h->runtime_proxy_) {
      ctx.callback->Failure(503, "runtime not connected");
      return;
    }
    const auto& jp = ctx.payload;
    auto extract = [&](std::string_view key) -> std::string {
      if (!jp.is_object())
        return {};
      auto it = jp.find(std::string(key));
      if (it == jp.end() || !it->is_string())
        return {};
      return it->get<std::string>();
    };
    const std::string run_id = extract("run_id");
    const std::string review_id = extract("review_id");
    const std::string body = extract("body");
    if (review_id.empty()) {
      ctx.callback->Failure(503, "missing review_id");
      return;
    }
    nlohmann::json req = {
        {"kind", "resolve_review"},
        {"run_id", run_id},
        {"review_id", review_id},
        {"decision", "approve"},
    };
    if (!body.empty())
      req["notes"] = body;
    h->runtime_proxy_->SendControl(
        std::move(req),
        [cb = ctx.callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            cb->Failure(500, resp.value("error", nlohmann::json{})
                                 .value("message", "approve failed"));
            return;
          }
          cb->Success(nlohmann::json{{"ok", true}});
        });
  });

  // ── review.request_changes ───────────────────────────────────────────────
  r.add("review.request_changes", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    if (!h->runtime_proxy_) {
      ctx.callback->Failure(503, "runtime not connected");
      return;
    }
    const auto& jp = ctx.payload;
    auto extract = [&](std::string_view key) -> std::string {
      if (!jp.is_object())
        return {};
      auto it = jp.find(std::string(key));
      if (it == jp.end() || !it->is_string())
        return {};
      return it->get<std::string>();
    };
    const std::string run_id = extract("run_id");
    const std::string review_id = extract("review_id");
    const std::string body = extract("body");
    if (review_id.empty()) {
      ctx.callback->Failure(503, "missing review_id");
      return;
    }
    nlohmann::json req = {
        {"kind", "resolve_review"},
        {"run_id", run_id},
        {"review_id", review_id},
        {"decision", "reject"},
    };
    if (!body.empty())
      req["notes"] = body;
    h->runtime_proxy_->SendControl(
        std::move(req),
        [cb = ctx.callback](nlohmann::json resp, bool is_error) {
          if (is_error) {
            cb->Failure(500, resp.value("error", nlohmann::json{})
                                 .value("message", "request_changes failed"));
            return;
          }
          cb->Success(nlohmann::json{{"ok", true}});
        });
  });

  // ── review.comment ───────────────────────────────────────────────────────
  r.add("review.comment", [h](BridgeCtx ctx) {
    auto* sp = h->space_manager_->ActiveSpace();
    if (!sp) {
      ctx.callback->Failure(503, "no active space");
      return;
    }
    if (!h->runtime_proxy_) {
      ctx.callback->Failure(503, "runtime not connected");
      return;
    }
    const auto& jp = ctx.payload;
    auto extract = [&](std::string_view key) -> std::string {
      if (!jp.is_object())
        return {};
      auto it = jp.find(std::string(key));
      if (it == jp.end() || !it->is_string())
        return {};
      return it->get<std::string>();
    };
    const std::string run_id = extract("run_id");
    const std::string review_id = extract("review_id");
    const std::string body = extract("body");
    const std::string name = extract("name");
    if (run_id.empty()) {
      ctx.callback->Failure(503, "missing run_id");
      return;
    }
    nlohmann::json comment_payload = {{"comment", body}};
    if (!review_id.empty())
      comment_payload["review_id"] = review_id;
    if (!name.empty())
      comment_payload["doc"] = name;
    nlohmann::json req = {
        {"kind", "post_input"},
        {"run_id", run_id},
        {"payload", std::move(comment_payload)},
    };
    h->runtime_proxy_->SendControl(
        std::move(req),
        [cb = ctx.callback](nlohmann::json resp, bool is_error) {
          cb->Success(nlohmann::json{{"ok", true}});
        });
  });
}

}  // namespace cronymax
