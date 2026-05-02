#ifndef CRONYMAX_FLOW_ROUTER_H_
#define CRONYMAX_FLOW_ROUTER_H_

#include <string>
#include <string_view>
#include <vector>

namespace cronymax {

class FlowDefinition;

// One scheduled downstream Agent invocation produced by the Router.
struct RouteTarget {
  std::string agent;
  // "typed-port"   = matched a Flow edge by producing port
  // "mention"      = matched an @mention in the document body
  // "typed+mention"= matched both (deduplicated; reported once)
  std::string reason;
};

// Routing decision returned by Router::Route.
struct RouteDecision {
  std::vector<RouteTarget> targets;
  // Mentions that didn't resolve to any declared agent in the Flow. Used
  // to surface a non-blocking warning in the trace stream.
  std::vector<std::string> unknown_mentions;
};

// Router combines typed-port routing (Flow edges) with @mention routing
// (free-text references inside the document body). Backward edges are
// allowed: an @mention to an upstream Agent is honoured.
class Router {
 public:
  // `flow` may not be null. `producing_agent` is the Agent that just
  // submitted; `port` is the doc-type the document was submitted as;
  // `body` is the document content (for mention parsing).
  static RouteDecision Route(const FlowDefinition& flow,
                             std::string_view producing_agent,
                             std::string_view port,
                             std::string_view body);
};

}  // namespace cronymax

#endif  // CRONYMAX_FLOW_ROUTER_H_
