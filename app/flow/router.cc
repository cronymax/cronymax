#include "flow/router.h"

#include <algorithm>
#include <unordered_map>
#include <unordered_set>

#include "flow/flow_definition.h"
#include "flow/mention_parser.h"

namespace cronymax {

RouteDecision Router::Route(const FlowDefinition& flow,
                            std::string_view producing_agent,
                            std::string_view port,
                            std::string_view body) {
  RouteDecision decision;

  // Set of declared agents — used both for mention validation and to
  // dedupe targets.
  std::unordered_set<std::string> known(flow.agents().begin(),
                                        flow.agents().end());

  // Stage 1: typed-port matches.
  std::unordered_map<std::string, std::string> reasons;  // agent -> reason
  for (const auto& edge : flow.edges()) {
    if (edge.from_agent != producing_agent) continue;
    if (edge.port != port) continue;
    reasons[edge.to_agent] = "typed-port";
  }

  // Stage 2: @mention matches against declared agents in this Flow.
  const auto mentions = MentionParser::Parse(body);
  std::unordered_set<std::string> seen_mentions;
  for (const auto& m : mentions) {
    if (!seen_mentions.insert(m.name).second) continue;
    if (known.find(m.name) == known.end()) {
      decision.unknown_mentions.push_back(m.name);
      continue;
    }
    auto it = reasons.find(m.name);
    if (it == reasons.end()) {
      reasons[m.name] = "mention";
    } else if (it->second == "typed-port") {
      it->second = "typed+mention";
    }
  }

  // Stage 3: emit targets in a stable order — typed first (declaration
  // order), then mention-only (parse order).
  std::unordered_set<std::string> emitted;
  for (const auto& edge : flow.edges()) {
    if (edge.from_agent != producing_agent || edge.port != port) continue;
    if (!emitted.insert(edge.to_agent).second) continue;
    auto it = reasons.find(edge.to_agent);
    if (it == reasons.end()) continue;
    decision.targets.push_back({edge.to_agent, it->second});
  }
  for (const auto& m : mentions) {
    if (known.find(m.name) == known.end()) continue;
    if (!emitted.insert(m.name).second) continue;
    auto it = reasons.find(m.name);
    if (it == reasons.end()) continue;
    decision.targets.push_back({m.name, it->second});
  }

  return decision;
}

}  // namespace cronymax
