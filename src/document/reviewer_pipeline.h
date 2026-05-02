#ifndef CRONYMAX_DOCUMENT_REVIEWER_PIPELINE_H_
#define CRONYMAX_DOCUMENT_REVIEWER_PIPELINE_H_

#include <chrono>
#include <functional>
#include <future>
#include <string>
#include <vector>

#include "document/schema_reviewer.h"

namespace cronymax {

class DocTypeSchema;

// Per-Flow reviewer policy.
struct ReviewerPolicy {
  // Maximum number of revision rounds before exhaustion handling kicks in.
  int max_review_rounds = 3;

  // What to do when max_review_rounds is reached and reviewers still
  // produce changes_requested findings.
  enum class OnExhausted {
    kApprove,  // auto-approve, mark review_exhausted=true
    kHalt,     // fail the run with reason "review_exhausted"
  } on_exhausted = OnExhausted::kApprove;

  // Per-reviewer timeout (LLM-driven reviewers only).
  std::chrono::seconds reviewer_timeout = std::chrono::seconds(60);

  // Whether a human approval gate is required after reviewers pass.
  bool require_human_gate = false;
};

// Identity + invoker for an LLM-backed reviewer agent. The invoker is
// supplied by the caller (typically wraps `AgentRuntime`) so this
// pipeline does not depend on the agent module directly.
//
// The invoker is expected to honor the `timeout` argument by aborting
// the underlying LLM call; the pipeline does not enforce it externally.
struct LlmReviewer {
  std::string agent_name;
  using Invoke = std::function<ReviewerVerdict(const std::string& doc_content,
                                               std::chrono::seconds timeout)>;
  Invoke invoke;
};

// Aggregated outcome of one review round.
struct PipelineOutcome {
  enum class Status {
    kApproved,            // all reviewers passed (or gate satisfied)
    kChangesRequested,    // at least one reviewer asked for changes
    kHumanGate,           // reviewers passed; awaiting human approval
    kReviewExhausted,     // round_count == max_review_rounds and policy=approve
    kHalt,                // round_count == max_review_rounds and policy=halt
  } status = Status::kChangesRequested;

  std::vector<ReviewerFinding> findings;  // aggregate across reviewers
};

class ReviewerPipeline {
 public:
  ReviewerPipeline(const DocTypeSchema& schema, ReviewerPolicy policy,
                   std::vector<LlmReviewer> llm_reviewers);

  // Run validators (schema first, blocking) then LLM reviewers in
  // parallel. `current_round` is the 1-based round index after this
  // submission (i.e. the new round_count). This call is synchronous and
  // can take up to `policy.reviewer_timeout` for the LLM stage.
  PipelineOutcome Run(const std::string& doc_content, int current_round) const;

 private:
  const DocTypeSchema& schema_;
  ReviewerPolicy policy_;
  std::vector<LlmReviewer> llm_reviewers_;
};

}  // namespace cronymax

#endif  // CRONYMAX_DOCUMENT_REVIEWER_PIPELINE_H_
