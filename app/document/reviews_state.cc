#include "document/reviews_state.h"

#include <sstream>

#include "common/json_value.h"

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

JsonValue RevisionToJson(const DocRevisionRecord& r) {
  auto j = JsonValue::Object();
  j.as_object()["rev"] = JsonValue::Number(r.rev);
  j.as_object()["submitted_at"] = JsonValue::String(r.submitted_at);
  j.as_object()["submitted_by"] = JsonValue::String(r.submitted_by);
  j.as_object()["sha"] = JsonValue::String(r.sha);
  return j;
}

JsonValue CommentToJson(const DocComment& c) {
  auto j = JsonValue::Object();
  j.as_object()["id"] = JsonValue::String(c.id);
  j.as_object()["author"] = JsonValue::String(c.author);
  j.as_object()["kind"] = JsonValue::String(c.kind);
  j.as_object()["anchor"] = JsonValue::String(c.anchor);
  j.as_object()["body"] = JsonValue::String(c.body);
  if (c.resolved_in_rev) {
    j.as_object()["resolved_in_rev"] = JsonValue::Number(*c.resolved_in_rev);
  }
  if (c.created_at_ms != 0) {
    j.as_object()["created_at_ms"] = JsonValue::Number(
        static_cast<double>(c.created_at_ms));
  }
  // Block-anchored comment fields. Emit only when non-empty so legacy
  // reviews.json files stay byte-for-byte unchanged on save.
  if (!c.block_id.empty()) {
    j.as_object()["block_id"] = JsonValue::String(c.block_id);
  }
  if (!c.suggestion.empty()) {
    j.as_object()["suggestion"] = JsonValue::String(c.suggestion);
  }
  if (!c.legacy_anchor.empty()) {
    j.as_object()["legacy_anchor"] = JsonValue::String(c.legacy_anchor);
  }
  return j;
}

JsonValue DocStateToJson(const DocReviewState& d) {
  auto j = JsonValue::Object();
  j.as_object()["current_revision"] = JsonValue::Number(d.current_revision);
  j.as_object()["status"] = JsonValue::String(DocStatusToString(d.status));
  j.as_object()["round_count"] = JsonValue::Number(d.round_count);
  if (d.review_exhausted) {
    j.as_object()["review_exhausted"] = JsonValue::Bool(true);
  }
  auto revs = JsonValue::Array();
  for (const auto& r : d.revisions) revs.as_array().push_back(RevisionToJson(r));
  j.as_object()["revisions"] = std::move(revs);
  auto cmts = JsonValue::Array();
  for (const auto& c : d.comments) cmts.as_array().push_back(CommentToJson(c));
  j.as_object()["comments"] = std::move(cmts);
  return j;
}

bool RevisionFromJson(const JsonValue& j, DocRevisionRecord* out,
                      std::string* error) {
  if (!j.is_object()) { if (error) *error = "revision: not object"; return false; }
  out->rev = j.Get("rev").as_int();
  if (j.Get("submitted_at").is_string()) out->submitted_at = j.Get("submitted_at").as_string();
  if (j.Get("submitted_by").is_string()) out->submitted_by = j.Get("submitted_by").as_string();
  if (j.Get("sha").is_string()) out->sha = j.Get("sha").as_string();
  return true;
}

bool CommentFromJson(const JsonValue& j, DocComment* out,
                     std::string* error) {
  if (!j.is_object()) { if (error) *error = "comment: not object"; return false; }
  if (j.Get("id").is_string()) out->id = j.Get("id").as_string();
  if (j.Get("author").is_string()) out->author = j.Get("author").as_string();
  if (j.Get("kind").is_string()) out->kind = j.Get("kind").as_string();
  if (j.Get("anchor").is_string()) out->anchor = j.Get("anchor").as_string();
  if (j.Get("body").is_string()) out->body = j.Get("body").as_string();
  if (j.Get("resolved_in_rev").is_number()) {
    out->resolved_in_rev = j.Get("resolved_in_rev").as_int();
  }
  if (j.Get("created_at_ms").is_number()) {
    out->created_at_ms = j.Get("created_at_ms").as_i64();
  }
  // Block-anchored comment fields (optional; empty if absent).
  if (j.Get("block_id").is_string()) out->block_id = j.Get("block_id").as_string();
  if (j.Get("suggestion").is_string()) out->suggestion = j.Get("suggestion").as_string();
  if (j.Get("legacy_anchor").is_string()) out->legacy_anchor = j.Get("legacy_anchor").as_string();
  return true;
}

bool DocStateFromJson(const JsonValue& j, DocReviewState* out,
                      std::string* error) {
  if (!j.is_object()) { if (error) *error = "doc state: not object"; return false; }
  out->current_revision = j.Get("current_revision").as_int();
  if (j.Get("status").is_string()) {
    ParseDocStatus(j.Get("status").as_string(), &out->status);
  }
  out->round_count = j.Get("round_count").as_int();
  if (j.Get("review_exhausted").is_bool()) {
    out->review_exhausted = j.Get("review_exhausted").as_bool();
  }
  if (j.Get("revisions").is_array()) {
    for (const auto& r : j.Get("revisions").as_array()) {
      DocRevisionRecord rec;
      if (!RevisionFromJson(r, &rec, error)) return false;
      out->revisions.push_back(std::move(rec));
    }
  }
  if (j.Get("comments").is_array()) {
    for (const auto& c : j.Get("comments").as_array()) {
      DocComment cmt;
      if (!CommentFromJson(c, &cmt, error)) return false;
      out->comments.push_back(std::move(cmt));
    }
  }
  return true;
}

}  // namespace

std::string ReviewsState::ToJson() const {
  auto root = JsonValue::Object();
  auto docs_obj = JsonValue::Object();
  for (const auto& kv : docs) {
    docs_obj.as_object()[kv.first] = DocStateToJson(kv.second);
  }
  root.as_object()["docs"] = std::move(docs_obj);
  return root.Dump(/*compact=*/false);
}

bool ReviewsState::FromJson(const std::string& json, ReviewsState* out,
                            std::string* error) {
  JsonValue root;
  if (!JsonValue::Parse(json, &root, error)) return false;
  if (!root.is_object()) {
    if (error) *error = "reviews.json: root must be object";
    return false;
  }
  const auto& docs_v = root.Get("docs");
  if (docs_v.is_null()) return true;  // empty state OK
  if (!docs_v.is_object()) {
    if (error) *error = "reviews.json: 'docs' must be object";
    return false;
  }
  for (const auto& kv : docs_v.as_object()) {
    DocReviewState ds;
    if (!DocStateFromJson(kv.second, &ds, error)) return false;
    out->docs.emplace(kv.first, std::move(ds));
  }
  return true;
}

}  // namespace cronymax
