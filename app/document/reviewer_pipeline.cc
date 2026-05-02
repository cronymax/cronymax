#include "document/reviewer_pipeline.h"

#include <future>
#include <memory>
#include <thread>
#include <utility>

#include "document/doc_type_schema.h"

namespace cronymax {

ReviewerPipeline::ReviewerPipeline(const DocTypeSchema& schema,
                                   ReviewerPolicy policy,
                                   std::vector<LlmReviewer> llm_reviewers)
    : schema_(schema),
      policy_(policy),
      llm_reviewers_(std::move(llm_reviewers)) {}

PipelineOutcome ReviewerPipeline::Run(const std::string& doc_content,
                                      int current_round) const {
  PipelineOutcome out;

  // Stage 1: deterministic schema reviewer (blocking).
  auto schema_v = SchemaReviewer::Review(schema_, doc_content);
  for (auto& f : schema_v.findings) out.findings.push_back(std::move(f));

  bool any_changes_requested = !schema_v.ok;

  // Stage 2: LLM-backed reviewers in parallel. Failures from a single
  // reviewer are demoted to a warning finding so they cannot stall the
  // pipeline (per design: reviewer timeout doesn't block the run).
  if (!llm_reviewers_.empty()) {
    std::vector<std::future<ReviewerVerdict>> futs;
    futs.reserve(llm_reviewers_.size());
    for (const auto& r : llm_reviewers_) {
      const auto timeout = policy_.reviewer_timeout;
      const auto& invoke = r.invoke;
      const auto agent_name = r.agent_name;
      // Use a detached std::thread + std::promise rather than
      // std::async, because std::async's returned future has a destructor
      // that BLOCKS on the worker thread — defeating the timeout we
      // designed (a runaway reviewer would still stall pipeline return).
      auto prom = std::make_shared<std::promise<ReviewerVerdict>>();
      futs.push_back(prom->get_future());
      std::thread([invoke, timeout, doc_content, agent_name, prom]() {
        if (!invoke) {
          ReviewerVerdict v;
          v.findings.push_back(
              {"comment", "reviewer:" + agent_name,
               "no invoker bound; reviewer skipped"});
          v.ok = true;
          prom->set_value(std::move(v));
          return;
        }
        prom->set_value(invoke(doc_content, timeout));
      }).detach();
    }
    // Wait at most timeout + small grace per reviewer; results outside
    // window are treated as a non-blocking warning.
    const auto deadline =
        std::chrono::steady_clock::now() + policy_.reviewer_timeout +
        std::chrono::seconds(2);
    for (std::size_t i = 0; i < futs.size(); ++i) {
      auto& f = futs[i];
      auto status = f.wait_until(deadline);
      if (status != std::future_status::ready) {
        out.findings.push_back(
            {"comment", "reviewer:" + llm_reviewers_[i].agent_name,
             "reviewer timed out; ignoring this round"});
        continue;
      }
      auto v = f.get();
      for (auto& fnd : v.findings) out.findings.push_back(std::move(fnd));
      if (!v.ok) any_changes_requested = true;
    }
  }

  // Stage 3: decide outcome.
  if (any_changes_requested) {
    if (current_round >= policy_.max_review_rounds) {
      switch (policy_.on_exhausted) {
        case ReviewerPolicy::OnExhausted::kApprove:
          out.status = PipelineOutcome::Status::kReviewExhausted;
          break;
        case ReviewerPolicy::OnExhausted::kHalt:
          out.status = PipelineOutcome::Status::kHalt;
          break;
      }
    } else {
      out.status = PipelineOutcome::Status::kChangesRequested;
    }
    return out;
  }

  // No reviewer asked for changes.
  if (policy_.require_human_gate) {
    out.status = PipelineOutcome::Status::kHumanGate;
  } else {
    out.status = PipelineOutcome::Status::kApproved;
  }
  return out;
}

}  // namespace cronymax
