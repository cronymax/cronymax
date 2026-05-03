#include "document/reviews_state.h"

#include <nlohmann/json.hpp>

namespace cronymax {

const char* DocStatusToString(DocStatus s) {
  switch (s) {
    case DocStatus::kDraft: return "DRAFT";
    case DocStatus::kInReview: return "IN_REVIEW";
    case DocStatus::kChangesRequested: return "CHANGES_REQUESTED";
    case DocStatus::kApproved: return "APPROVED";
    case DocStatus::kHandedOff: return "HANDED_OFF";
  }
  return "DRAFT";
}

bool ParseDocStatus(const std::string& s, DocStatus* out) {
  if (s == "DRAFT") { *out = DocStatus::kDraft; return true; }
  if (s == "IN_REVIEW") { *out = DocStatus::kInReview; return true; }
  if (s == "CHANGES_REQUESTED") { *out = DocStatus::kChangesRequested; return true; }
  if (s == "APPROVED") { *out = DocStatus::kApproved; return true; }
  if (s == "HANDED_OFF") { *out = DocStatus::kHandedOff; return true; }
  return false;
}

namespace {

nlohmann::json RevisionToJson(const DocRevisionRecord& r) {
  return {
    {"rev",          r.rev},
    {"submitted_at", r.submitted_at},
    {"submitted_by", r.submitted_by},
    {"sha",          r.sha},
  };
}

nlohmann::json CommentToJson(const DocComment& c) {
  nlohmann::json j = {
    {"id",     c.id},
    {"author", c.author},
    {"kind",   c.kind},
    {"anchor", c.anchor},
    {"body",   c.body},
  };
  if (c.resolved_in_rev) j["resolved_in_rev"] = *c.resolved_in_rev;
  if (c.created_at_ms != 0) j["created_at_ms"] = c.created_at_ms;
  // Block-anchored comment fields. Emit only when non-empty so legacy
  // reviews.json files stay byte-for-byte unchanged on save.
  if (!c.block_id.empty())      j["block_id"]      = c.block_id;
  if (!c.suggestion.empty())    j["suggestion"]    = c.suggestion;
  if (!c.legacy_anchor.empty()) j["legacy_anchor"] = c.legacy_anchor;
  return j;
}

nlohmann::json DocStateToJson(const DocReviewState& d) {
  nlohmann::json j = {
    {"current_revision", d.current_revision},
    {"status",           DocStatusToString(d.status)},
    {"round_count",      d.round_count},
  };
  if (d.review_exhausted) j["review_exhausted"] = true;
  nlohmann::json revs = nlohmann::json::array();
  for (const auto& r : d.revisions) revs.push_back(RevisionToJson(r));
  j["revisions"] = std::move(revs);
  nlohmann::json cmts = nlohmann::json::array();
  for (const auto& c : d.comments) cmts.push_back(CommentToJson(c));
  j["comments"] = std::move(cmts);
  return j;
}

bool RevisionFromJson(const nlohmann::json& j, DocRevisionRecord* out,
                      std::string* error) {
  if (!j.is_object()) { if (error) *error = "revision: not object"; return false; }
  if (j.contains("rev") && j["rev"].is_number())
    out->rev = j["rev"].get<int>();
  if (j.contains("submitted_at") && j["submitted_at"].is_string())
    out->submitted_at = j["submitted_at"].get<std::string>();
  if (j.contains("submitted_by") && j["submitted_by"].is_string())
    out->submitted_by = j["submitted_by"].get<std::string>();
  if (j.contains("sha") && j["sha"].is_string())
    out->sha = j["sha"].get<std::string>();
  return true;
}

bool CommentFromJson(const nlohmann::json& j, DocComment* out,
                     std::string* error) {
  if (!j.is_object()) { if (error) *error = "comment: not object"; return false; }
  auto str = [&](const char* k, std::string* dst) {
    if (j.contains(k) && j[k].is_string()) *dst = j[k].get<std::string>();
  };
  str("id",     &out->id);
  str("author", &out->author);
  str("kind",   &out->kind);
  str("anchor", &out->anchor);
  str("body",   &out->body);
  if (j.contains("resolved_in_rev") && j["resolved_in_rev"].is_number())
    out->resolved_in_rev = j["resolved_in_rev"].get<int>();
  if (j.contains("created_at_ms") && j["created_at_ms"].is_number())
    out->created_at_ms = j["created_at_ms"].get<long long>();
  str("block_id",      &out->block_id);
  str("suggestion",    &out->suggestion);
  str("legacy_anchor", &out->legacy_anchor);
  return true;
}

bool DocStateFromJson(const nlohmann::json& j, DocReviewState* out,
                      std::string* error) {
  if (!j.is_object()) { if (error) *error = "doc state: not object"; return false; }
  if (j.contains("current_revision") && j["current_revision"].is_number())
    out->current_revision = j["current_revision"].get<int>();
  if (j.contains("status") && j["status"].is_string())
    ParseDocStatus(j["status"].get<std::string>(), &out->status);
  if (j.contains("round_count") && j["round_count"].is_number())
    out->round_count = j["round_count"].get<int>();
  if (j.contains("review_exhausted") && j["review_exhausted"].is_boolean())
    out->review_exhausted = j["review_exhausted"].get<bool>();
  if (j.contains("revisions") && j["revisions"].is_array()) {
    for (const auto& r : j["revisions"]) {
      DocRevisionRecord rec;
      if (!RevisionFromJson(r, &rec, error)) return false;
      out->revisions.push_back(std::move(rec));
    }
  }
  if (j.contains("comments") && j["comments"].is_array()) {
    for (const auto& c : j["comments"]) {
      DocComment cmt;
      if (!CommentFromJson(c, &cmt, error)) return false;
      out->comments.push_back(std::move(cmt));
    }
  }
  return true;
}

}  // namespace

std::string ReviewsState::ToJson() const {
  nlohmann::json docs_obj = nlohmann::json::object();
  for (const auto& kv : docs) {
    docs_obj[kv.first] = DocStateToJson(kv.second);
  }
  nlohmann::json root = {{"docs", std::move(docs_obj)}};
  return root.dump(2);
}

bool ReviewsState::FromJson(const std::string& json, ReviewsState* out,
                            std::string* error) {
  nlohmann::json root;
  root = nlohmann::json::parse(json, nullptr, false);
  if (root.is_discarded()) {
    if (error) *error = "JSON parse error";
    return false;
  }
  if (!root.is_object()) {
    if (error) *error = "reviews.json: root must be object";
    return false;
  }
  if (!root.contains("docs")) return true;  // empty state OK
  if (!root["docs"].is_object()) {
    if (error) *error = "reviews.json: 'docs' must be object";
    return false;
  }
  for (const auto& [key, val] : root["docs"].items()) {
    DocReviewState ds;
    if (!DocStateFromJson(val, &ds, error)) return false;
    out->docs.emplace(key, std::move(ds));
  }
  return true;
}

}  // namespace cronymax
