//! State graph — LangGraph-style declarative agent graph with nodes and edges.
//!
//! This is the centerpiece of the framework: a typed, compilable graph where
//! **nodes** are agent actions and **edges** define transitions between them.
//!
//! # Usage
//!
//! ```rust,ignore
//! use cronygraph::graph::{StateGraph, END};
//!
//! let mut graph = StateGraph::new();
//! graph.add_node("planner", planner_node);
//! graph.add_node("researcher", researcher_node);
//! graph.add_node("writer", writer_node);
//!
//! graph.add_edge("planner", "researcher");
//! graph.add_conditional_edge("researcher", router);
//! graph.add_edge("writer", END);
//! graph.set_entry("planner");
//!
//! let compiled = graph.compile()?;
//! let result = compiled.run(state, &llm_factory).await?;
//! ```
//!
//! # DeerFlow Patterns
//!
//! - **Supervisor graph**: A supervisor node routes to worker nodes and collects results
//! - **Research flow**: search → read → summarize → synthesize pipeline
//! - **Reflection loop**: agent → critic → agent (iterative refinement)

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::checkpoint::{CheckpointAction, CheckpointHandler};
use crate::engine::{LlmBackendFactory, SequentialToolExecutor};
use crate::node::AgentNode;
use crate::state::{OrchestrationState, SubAgentResult};
use crate::types::{ChatMessage, MessageImportance, MessageRole};

/// Sentinel name indicating the graph should terminate.
pub const END: &str = "__end__";

// ─── GraphNode Trait ─────────────────────────────────────────────────────────

/// A node in the state graph — executes an action and returns the next node name.
#[async_trait]
pub trait GraphNode: Send + Sync {
    /// Execute this node, possibly modifying the orchestration state.
    ///
    /// Returns the name of the next node to transition to, or [`END`] to finish.
    /// For nodes with outgoing conditional edges, the return value is ignored
    /// and the router determines the next step instead.
    async fn execute(
        &self,
        state: &mut OrchestrationState,
        llm_factory: &dyn LlmBackendFactory,
    ) -> anyhow::Result<String>;
}

// ─── AgentGraphNode ──────────────────────────────────────────────────────────

/// A graph node backed by an [`AgentNode`] — runs the agent loop to completion.
pub struct AgentGraphNode {
    pub agent: AgentNode,
}

impl AgentGraphNode {
    pub fn new(agent: AgentNode) -> Self {
        Self { agent }
    }
}

#[async_trait]
impl GraphNode for AgentGraphNode {
    async fn execute(
        &self,
        state: &mut OrchestrationState,
        llm_factory: &dyn LlmBackendFactory,
    ) -> anyhow::Result<String> {
        let llm = llm_factory.create(self.agent.model.as_deref().unwrap_or("gpt-4o"));
        let result = self
            .agent
            .run(&*llm, &SequentialToolExecutor, state.messages.clone())
            .await?;

        state.sub_results.insert(
            self.agent.name.clone(),
            SubAgentResult {
                response: result.response.clone(),
                usage: result.total_usage.clone(),
            },
        );
        state.accumulate_usage(result.total_usage);

        // Append the agent's response as an assistant message.
        state.messages.push(ChatMessage::new(
            MessageRole::Assistant,
            result.response,
            MessageImportance::Normal,
            0,
        ));

        Ok(END.to_string())
    }
}

// ─── Edge Types ──────────────────────────────────────────────────────────────

/// Edge transition in the state graph.
enum Edge {
    /// Always go to a fixed node.
    Direct(String),
    /// Route dynamically based on state.
    Conditional(Box<dyn ConditionalRouter>),
}

/// A router that decides the next node based on orchestration state.
///
/// This is the graph-level equivalent of LangGraph's conditional edges.
#[async_trait]
pub trait ConditionalRouter: Send + Sync {
    /// Given the current state and the node that just executed, return the
    /// name of the next node to run, or [`END`] to terminate.
    async fn route(&self, state: &OrchestrationState, current_node: &str)
    -> anyhow::Result<String>;
}

type RoutableFunc = Arc<dyn Fn(&OrchestrationState, &str) -> String + Send + Sync>;

/// Simple function-based conditional router.
pub struct FnRouter {
    func: RoutableFunc,
}

impl FnRouter {
    pub fn new(func: impl Fn(&OrchestrationState, &str) -> String + Send + Sync + 'static) -> Self {
        Self {
            func: Arc::new(func),
        }
    }
}

#[async_trait]
impl ConditionalRouter for FnRouter {
    async fn route(
        &self,
        state: &OrchestrationState,
        current_node: &str,
    ) -> anyhow::Result<String> {
        Ok((self.func)(state, current_node))
    }
}

// ─── StateGraph Builder ──────────────────────────────────────────────────────

/// Declarative state graph builder — LangGraph-style API.
///
/// Define nodes and edges, then compile into a runnable [`CompiledGraph`].
pub struct StateGraph {
    nodes: HashMap<String, Box<dyn GraphNode>>,
    edges: HashMap<String, Edge>,
    entry: Option<String>,
    /// Optional checkpoint handler invoked before each node execution.
    checkpoint: Option<CheckpointHandler>,
}

impl StateGraph {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: HashMap::new(),
            entry: None,
            checkpoint: None,
        }
    }

    /// Add a named node to the graph.
    pub fn add_node(&mut self, name: impl Into<String>, node: impl GraphNode + 'static) {
        self.nodes.insert(name.into(), Box::new(node));
    }

    /// Add a direct edge: after `from` completes, always go to `to`.
    pub fn add_edge(&mut self, from: impl Into<String>, to: impl Into<String>) {
        self.edges.insert(from.into(), Edge::Direct(to.into()));
    }

    /// Add a conditional edge: after `from` completes, the router decides next.
    pub fn add_conditional_edge(
        &mut self,
        from: impl Into<String>,
        router: impl ConditionalRouter + 'static,
    ) {
        self.edges
            .insert(from.into(), Edge::Conditional(Box::new(router)));
    }

    /// Set the entry point node name.
    pub fn set_entry(&mut self, name: impl Into<String>) {
        self.entry = Some(name.into());
    }

    /// Set a checkpoint handler invoked before each node execution.
    pub fn set_checkpoint(&mut self, handler: CheckpointHandler) {
        self.checkpoint = Some(handler);
    }

    /// Compile the graph into a runnable form. Validates structure.
    pub fn compile(self) -> anyhow::Result<CompiledGraph> {
        let entry = self
            .entry
            .ok_or_else(|| anyhow::anyhow!("StateGraph has no entry point — call set_entry()"))?;

        if !self.nodes.contains_key(&entry) {
            return Err(anyhow::anyhow!(
                "Entry point '{}' not found in graph nodes",
                entry
            ));
        }

        // Validate all edge targets exist.
        for (from, edge) in &self.edges {
            if !self.nodes.contains_key(from) {
                return Err(anyhow::anyhow!(
                    "Edge source '{}' not found in graph nodes",
                    from
                ));
            }
            if let Edge::Direct(to) = edge
                && to != END
                && !self.nodes.contains_key(to)
            {
                return Err(anyhow::anyhow!(
                    "Edge target '{}' not found in graph nodes",
                    to
                ));
            }
        }

        Ok(CompiledGraph {
            nodes: self.nodes,
            edges: self.edges,
            entry,
            checkpoint: self.checkpoint,
        })
    }
}

impl Default for StateGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ─── CompiledGraph ───────────────────────────────────────────────────────────

/// A compiled, validated state graph ready for execution.
///
/// Walks through nodes following edges until reaching [`END`] or a node
/// with no outgoing edge.
pub struct CompiledGraph {
    nodes: HashMap<String, Box<dyn GraphNode>>,
    edges: HashMap<String, Edge>,
    entry: String,
    checkpoint: Option<CheckpointHandler>,
}

impl CompiledGraph {
    /// Execute the graph from the entry point to completion.
    ///
    /// Returns the final orchestration state after all nodes have run.
    pub async fn run(
        &self,
        state: &mut OrchestrationState,
        llm_factory: &dyn LlmBackendFactory,
    ) -> anyhow::Result<()> {
        let mut current = self.entry.clone();
        let mut iterations = 0;
        let max_iterations = 100; // Safety limit against infinite loops.

        loop {
            if current == END || iterations >= max_iterations {
                break;
            }
            iterations += 1;

            let node = self
                .nodes
                .get(&current)
                .ok_or_else(|| anyhow::anyhow!("Node '{}' not found during execution", current))?;

            // ── Checkpoint before node execution ─────────────────────
            if let Some(ref handler) = self.checkpoint {
                match handler(state) {
                    CheckpointAction::Continue => {}
                    CheckpointAction::Abort(reason) => {
                        log::info!("[StateGraph] Aborted at '{}': {}", current, reason);
                        return Ok(());
                    }
                    CheckpointAction::ModifyPlan(new_plan) => {
                        state.plan = Some(new_plan);
                    }
                }
            }

            log::info!("[StateGraph] Executing node '{}'", current);

            // ── Execute the node ─────────────────────────────────────
            let node_result = node.execute(state, llm_factory).await?;

            // ── Determine next step ──────────────────────────────────
            current = match self.edges.get(&current) {
                Some(Edge::Direct(to)) => to.clone(),
                Some(Edge::Conditional(router)) => router.route(state, &current).await?,
                None => {
                    // No outgoing edge — use the node's own return value.
                    node_result
                }
            };
        }

        if iterations >= max_iterations {
            log::warn!("[StateGraph] Hit max iterations ({})", max_iterations);
        }

        Ok(())
    }
}

// ─── Pre-built Graph Patterns (DeerFlow) ─────────────────────────────────────

/// Build a supervisor graph — a common DeerFlow pattern.
///
/// The supervisor node routes to worker nodes, collects their results,
/// and produces a final synthesis. This is the "plan → delegate → synthesize"
/// pattern as a declarative graph.
///
/// ```text
///           ┌──────────────┐
///  entry →  │  supervisor  │ ──conditional──→ worker_1
///           └──────────────┘                  worker_2
///                  ↑                          worker_3
///                  └──── all workers ──────────┘
/// ```
pub fn build_supervisor_graph(
    supervisor: AgentNode,
    workers: Vec<AgentNode>,
    router: impl ConditionalRouter + 'static,
) -> StateGraph {
    let mut graph = StateGraph::new();

    let supervisor_name = supervisor.name.clone();
    graph.add_node(&supervisor_name, AgentGraphNode::new(supervisor));

    for worker in workers {
        let worker_name = worker.name.clone();
        graph.add_node(&worker_name, AgentGraphNode::new(worker));
        // Workers return to supervisor after completion.
        graph.add_edge(&worker_name, &supervisor_name);
    }

    // Supervisor uses the router to pick the next worker (or END).
    graph.add_conditional_edge(&supervisor_name, router);
    graph.set_entry(&supervisor_name);

    graph
}

/// Build a linear pipeline graph — each agent feeds into the next.
///
/// ```text
/// entry → agent_1 → agent_2 → ... → agent_n → END
/// ```
pub fn build_pipeline_graph(agents: Vec<AgentNode>) -> StateGraph {
    let mut graph = StateGraph::new();

    if agents.is_empty() {
        return graph;
    }

    let first_name = agents[0].name.clone();
    let mut prev_name = String::new();

    for (i, agent) in agents.into_iter().enumerate() {
        let name = agent.name.clone();
        graph.add_node(&name, AgentGraphNode::new(agent));

        if i > 0 {
            graph.add_edge(&prev_name, &name);
        }
        prev_name = name;
    }

    // Last node → END
    graph.add_edge(&prev_name, END);
    graph.set_entry(&first_name);

    graph
}

/// Build a reflection loop graph — agent produces output, critic reviews,
/// and the agent refines. Repeats until the critic approves.
///
/// ```text
///           ┌─────────┐     ┌─────────┐
///  entry →  │  agent   │ ──→ │  critic  │ ──conditional──→ agent (retry)
///           └─────────┘     └─────────┘                   or END (approved)
/// ```
pub fn build_reflection_graph(
    agent: AgentNode,
    critic: AgentNode,
    should_retry: impl Fn(&OrchestrationState, &str) -> String + Send + Sync + 'static,
) -> StateGraph {
    let mut graph = StateGraph::new();

    let agent_name = agent.name.clone();
    let critic_name = critic.name.clone();

    graph.add_node(&agent_name, AgentGraphNode::new(agent));
    graph.add_node(&critic_name, AgentGraphNode::new(critic));

    // Agent always goes to critic for review.
    graph.add_edge(&agent_name, &critic_name);

    // Critic conditionally routes back to agent or to END.
    graph.add_conditional_edge(&critic_name, FnRouter::new(should_retry));

    graph.set_entry(&agent_name);
    graph
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{AgentLoopConfig, LlmBackend, LlmResult};
    use crate::types::TokenUsage;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FixedResponseNode {
        response: String,
        next: String,
    }

    #[async_trait]
    impl GraphNode for FixedResponseNode {
        async fn execute(
            &self,
            state: &mut OrchestrationState,
            _llm_factory: &dyn LlmBackendFactory,
        ) -> anyhow::Result<String> {
            state.sub_results.insert(
                "fixed".into(),
                SubAgentResult {
                    response: self.response.clone(),
                    usage: None,
                },
            );
            Ok(self.next.clone())
        }
    }

    struct CountingNode {
        name: String,
        counter: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl GraphNode for CountingNode {
        async fn execute(
            &self,
            state: &mut OrchestrationState,
            _llm_factory: &dyn LlmBackendFactory,
        ) -> anyhow::Result<String> {
            self.counter.fetch_add(1, Ordering::SeqCst);
            state.sub_results.insert(
                self.name.clone(),
                SubAgentResult {
                    response: format!("{} executed", self.name),
                    usage: None,
                },
            );
            Ok(END.to_string())
        }
    }

    struct DummyFactory;

    #[async_trait]
    impl LlmBackendFactory for DummyFactory {
        fn create(&self, _model: &str) -> Box<dyn LlmBackend> {
            struct DummyBackend;
            #[async_trait]
            impl LlmBackend for DummyBackend {
                async fn complete(
                    &self,
                    _messages: &[ChatMessage],
                    _tools: Option<&[serde_json::Value]>,
                ) -> anyhow::Result<LlmResult> {
                    Ok(LlmResult {
                        response: "dummy".into(),
                        tool_calls: vec![],
                        usage: Some(TokenUsage {
                            prompt_tokens: 1,
                            completion_tokens: 1,
                            total_tokens: 2,
                        }),
                    })
                }
            }
            Box::new(DummyBackend)
        }
    }

    #[test]
    fn compile_rejects_missing_entry() {
        let graph = StateGraph::new();
        assert!(graph.compile().is_err());
    }

    #[test]
    fn compile_rejects_unknown_entry() {
        let mut graph = StateGraph::new();
        graph.set_entry("nonexistent");
        assert!(graph.compile().is_err());
    }

    #[test]
    fn compile_rejects_bad_edge_target() {
        let mut graph = StateGraph::new();
        graph.add_node(
            "a",
            FixedResponseNode {
                response: "hello".into(),
                next: END.into(),
            },
        );
        graph.add_edge("a", "nonexistent");
        graph.set_entry("a");
        assert!(graph.compile().is_err());
    }

    #[test]
    fn compile_accepts_end_edge() {
        let mut graph = StateGraph::new();
        graph.add_node(
            "a",
            FixedResponseNode {
                response: "hello".into(),
                next: END.into(),
            },
        );
        graph.add_edge("a", END);
        graph.set_entry("a");
        assert!(graph.compile().is_ok());
    }

    #[tokio::test]
    async fn single_node_graph_runs_to_completion() {
        let mut graph = StateGraph::new();
        graph.add_node(
            "a",
            FixedResponseNode {
                response: "done".into(),
                next: END.into(),
            },
        );
        graph.set_entry("a");

        let compiled = graph.compile().unwrap();
        let mut state = OrchestrationState::new(vec![], 3);
        compiled.run(&mut state, &DummyFactory).await.unwrap();

        assert_eq!(state.sub_results["fixed"].response, "done");
    }

    #[tokio::test]
    async fn pipeline_graph_runs_all_nodes() {
        let counter = Arc::new(AtomicUsize::new(0));

        let mut graph = StateGraph::new();
        graph.add_node(
            "step1",
            CountingNode {
                name: "step1".into(),
                counter: counter.clone(),
            },
        );
        graph.add_node(
            "step2",
            CountingNode {
                name: "step2".into(),
                counter: counter.clone(),
            },
        );
        graph.add_edge("step1", "step2");
        graph.add_edge("step2", END);
        graph.set_entry("step1");

        let compiled = graph.compile().unwrap();
        let mut state = OrchestrationState::new(vec![], 3);
        compiled.run(&mut state, &DummyFactory).await.unwrap();

        assert_eq!(counter.load(Ordering::SeqCst), 2);
        assert!(state.sub_results.contains_key("step1"));
        assert!(state.sub_results.contains_key("step2"));
    }

    #[tokio::test]
    async fn conditional_edge_routes_correctly() {
        let counter_a = Arc::new(AtomicUsize::new(0));
        let counter_b = Arc::new(AtomicUsize::new(0));

        let mut graph = StateGraph::new();
        graph.add_node(
            "router_node",
            FixedResponseNode {
                response: "routing".into(),
                next: END.into(),
            },
        );
        graph.add_node(
            "target_a",
            CountingNode {
                name: "target_a".into(),
                counter: counter_a.clone(),
            },
        );
        graph.add_node(
            "target_b",
            CountingNode {
                name: "target_b".into(),
                counter: counter_b.clone(),
            },
        );

        // Always route to target_b.
        graph.add_conditional_edge(
            "router_node",
            FnRouter::new(|_state, _node| "target_b".to_string()),
        );
        graph.add_edge("target_a", END);
        graph.add_edge("target_b", END);
        graph.set_entry("router_node");

        let compiled = graph.compile().unwrap();
        let mut state = OrchestrationState::new(vec![], 3);
        compiled.run(&mut state, &DummyFactory).await.unwrap();

        assert_eq!(counter_a.load(Ordering::SeqCst), 0);
        assert_eq!(counter_b.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn reflection_loop_iterates() {
        let iteration_count = Arc::new(AtomicUsize::new(0));
        let ic = iteration_count.clone();

        let mut graph = StateGraph::new();
        graph.add_node(
            "worker",
            CountingNode {
                name: "worker".into(),
                counter: iteration_count.clone(),
            },
        );
        graph.add_node(
            "critic",
            CountingNode {
                name: "critic".into(),
                counter: iteration_count.clone(),
            },
        );
        graph.add_edge("worker", "critic");

        // Retry twice, then approve.
        graph.add_conditional_edge(
            "critic",
            FnRouter::new(move |_state, _node| {
                let count = ic.load(Ordering::SeqCst);
                if count < 4 {
                    // 2 iterations × 2 nodes = 4
                    "worker".to_string()
                } else {
                    END.to_string()
                }
            }),
        );
        graph.set_entry("worker");

        let compiled = graph.compile().unwrap();
        let mut state = OrchestrationState::new(vec![], 3);
        compiled.run(&mut state, &DummyFactory).await.unwrap();

        assert_eq!(iteration_count.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn build_pipeline_graph_constructs_valid_graph() {
        let agents = vec![
            AgentNode::new("step1", "sys1", AgentLoopConfig::default()),
            AgentNode::new("step2", "sys2", AgentLoopConfig::default()),
            AgentNode::new("step3", "sys3", AgentLoopConfig::default()),
        ];
        let graph = build_pipeline_graph(agents);
        assert!(graph.compile().is_ok());
    }

    #[test]
    fn build_supervisor_graph_constructs_valid_graph() {
        let supervisor = AgentNode::new("supervisor", "direct workers", AgentLoopConfig::default());
        let workers = vec![
            AgentNode::new("researcher", "search the web", AgentLoopConfig::default()),
            AgentNode::new("writer", "write content", AgentLoopConfig::default()),
        ];
        let router = FnRouter::new(|_state, _node| END.to_string());
        let graph = build_supervisor_graph(supervisor, workers, router);
        assert!(graph.compile().is_ok());
    }
}
