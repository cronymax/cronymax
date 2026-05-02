//! Orchestration primitives — a tiny, host-agnostic engine that walks
//! a [`crate::graph::Graph`] of [`Step`]s with a typed shared state.
//!
//! ## Scope (and what is intentionally excluded)
//!
//! `cronygraph` is the reusable orchestration engine. It exposes:
//!
//!   * [`Step`] — the trait every node implements; given the current
//!     `S`, it returns a [`Transition`] describing what to do next.
//!   * [`Transition`] — `Goto`, `Branch`, or `Terminal`.
//!   * [`TerminalReason`] — a small, generic vocabulary
//!     (`Completed` / `Cancelled` / `Custom(String)`). Product-specific
//!     reasons (e.g. "awaiting permission") are encoded by callers in
//!     `Custom(...)`; this crate must not learn product policy.
//!   * [`Orchestrator`] — drives the walk synchronously. Async,
//!     persistence, capability dispatch, LLM streaming, etc. live in
//!     `cronymax` which composes this engine.
//!   * [`RunOutcome`] — what the run produced: final state, terminal
//!     reason, and the visit trace.
//!   * [`StepLimits`] — caller-supplied guardrails (max steps); the
//!     engine itself takes no opinion on timeouts or scheduling.
//!
//! Everything is generic over `S` (state) and `E` (per-step error).
//! The crate has zero awareness of agents, LLM providers, host
//! capabilities, or persistence backends. That keeps the orchestration
//! layer reusable across whichever surface `cronymax` builds on top.

use thiserror::Error;

use crate::graph::{Graph, GraphError, NodeId};

/// What a step decided to do once it finished executing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition<S> {
    /// Move to a single named successor with the updated state.
    Goto { state: S, next: NodeId },
    /// Fan out to several successors. The orchestrator visits them in
    /// the supplied order; product-level concurrency belongs to the
    /// caller (cronymax) — this engine stays sequential.
    Branch { state: S, next: Vec<NodeId> },
    /// Stop the run. Carries the final state and a reason.
    Terminal { state: S, reason: TerminalReason },
}

/// Reasons a run can stop. Intentionally minimal; product-specific
/// pause/await semantics belong in `Custom(...)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalReason {
    Completed,
    Cancelled,
    /// Caller-defined reason (e.g. "awaiting-approval", "max-tokens").
    Custom(String),
}

/// A single orchestration node. Receives the current state by value
/// and returns a transition that names the next move.
pub trait Step<S, E>: Send + Sync {
    /// Optional human-readable label for traces.
    fn label(&self) -> &str {
        ""
    }
    fn execute(&self, state: S) -> Result<Transition<S>, E>;
}

/// Boxed step type used inside the orchestrator's graph payload.
pub type BoxedStep<S, E> = Box<dyn Step<S, E>>;

/// Closure-based [`Step`] adapter. Convenience for tests and small
/// inline graph definitions; production callers usually implement the
/// trait directly so they can carry per-node configuration.
pub struct FnStep<S, E, F>
where
    F: Fn(S) -> Result<Transition<S>, E> + Send + Sync,
{
    label: String,
    f: F,
    _state: std::marker::PhantomData<fn(S) -> S>,
    _err: std::marker::PhantomData<fn() -> E>,
}

impl<S, E, F> std::fmt::Debug for FnStep<S, E, F>
where
    F: Fn(S) -> Result<Transition<S>, E> + Send + Sync,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FnStep").field("label", &self.label).finish()
    }
}

impl<S, E, F> FnStep<S, E, F>
where
    F: Fn(S) -> Result<Transition<S>, E> + Send + Sync,
{
    pub fn new(label: impl Into<String>, f: F) -> Self {
        Self {
            label: label.into(),
            f,
            _state: std::marker::PhantomData,
            _err: std::marker::PhantomData,
        }
    }
}

impl<S, E, F> Step<S, E> for FnStep<S, E, F>
where
    F: Fn(S) -> Result<Transition<S>, E> + Send + Sync,
{
    fn label(&self) -> &str {
        &self.label
    }
    fn execute(&self, state: S) -> Result<Transition<S>, E> {
        (self.f)(state)
    }
}

/// Caller-supplied guardrails for a single run. The engine never
/// imposes time-based budgets — those are a host concern (cronymax can
/// wrap `Orchestrator::run` in a `tokio::time::timeout`).
#[derive(Debug, Clone, Copy)]
pub struct StepLimits {
    /// Hard cap on visited steps; protects against infinite loops in
    /// cyclic graphs. `None` means unlimited (cronymax may opt in to
    /// this for explicitly bounded DAGs).
    pub max_steps: Option<usize>,
}

impl Default for StepLimits {
    fn default() -> Self {
        Self {
            max_steps: Some(10_000),
        }
    }
}

/// Outcome of a completed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome<S> {
    pub final_state: S,
    pub reason: TerminalReason,
    /// Ordered list of visited node ids. Useful for tests, traces, and
    /// debugging without forcing the engine to know about logging.
    pub trace: Vec<NodeId>,
}

/// Errors the orchestrator can surface during a walk.
#[derive(Debug, Error)]
pub enum RunError<E> {
    #[error("graph error: {0}")]
    Graph(#[from] GraphError),
    #[error("step `{node}` returned error: {source}")]
    Step { node: NodeId, source: E },
    #[error("run exceeded max_steps={limit}")]
    StepLimitExceeded { limit: usize },
    #[error("transition referenced node {node} which is not a declared successor")]
    InvalidNextNode { node: NodeId },
    #[error("orchestrator has no entry node configured")]
    NoEntryNode,
}

/// Orchestration engine: owns the graph plus an entry node and walks
/// it for a given initial state.
pub struct Orchestrator<S, E> {
    graph: Graph<BoxedStep<S, E>, ()>,
    entry: Option<NodeId>,
}

impl<S, E> std::fmt::Debug for Orchestrator<S, E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Orchestrator")
            .field("nodes", &self.graph.len())
            .field("entry", &self.entry)
            .finish_non_exhaustive()
    }
}

impl<S, E> Default for Orchestrator<S, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, E> Orchestrator<S, E> {
    pub fn new() -> Self {
        Self {
            graph: Graph::new(),
            entry: None,
        }
    }

    /// Register a step and return its node id.
    pub fn add_step<T: Step<S, E> + 'static>(&mut self, step: T) -> NodeId {
        self.graph.add_node(Box::new(step))
    }

    /// Declare a permitted transition `from -> to`. Transitions that
    /// reference nodes without a declared edge are rejected at run
    /// time with [`RunError::InvalidNextNode`] — this lets the
    /// orchestrator catch typos in step output without forcing the
    /// graph to encode every conceivable jump.
    pub fn allow_transition(&mut self, from: NodeId, to: NodeId) -> Result<(), GraphError> {
        self.graph.add_edge(from, to, ())
    }

    pub fn set_entry(&mut self, entry: NodeId) {
        self.entry = Some(entry);
    }

    pub fn graph(&self) -> &Graph<BoxedStep<S, E>, ()> {
        &self.graph
    }

    /// Drive the orchestration synchronously.
    ///
    /// `Branch` transitions are walked in the order they were
    /// returned, depth-first: the engine pushes the remaining
    /// successors onto a work stack and drains them after the current
    /// branch terminates. Branching never introduces concurrency in
    /// this layer — concurrency is a host concern.
    pub fn run(&self, mut state: S, limits: StepLimits) -> Result<RunOutcome<S>, RunError<E>>
    where
        S: Clone,
    {
        let entry = self.entry.ok_or(RunError::NoEntryNode)?;
        if !self.graph.contains(entry) {
            return Err(RunError::Graph(GraphError::UnknownNode(entry)));
        }

        let mut trace = Vec::new();
        // Pending branch tails: (state_snapshot, node_id). When the
        // current path terminates we resume from the most recently
        // pushed entry. Depth-first ordering matches Branch::next.
        let mut pending: Vec<(S, NodeId)> = Vec::new();
        let mut current: NodeId = entry;
        let mut visited_count = 0usize;

        loop {
            if let Some(limit) = limits.max_steps {
                if visited_count >= limit {
                    return Err(RunError::StepLimitExceeded { limit });
                }
            }
            visited_count += 1;
            trace.push(current);

            let step = self
                .graph
                .node(current)
                .ok_or(RunError::Graph(GraphError::UnknownNode(current)))?;
            let transition = step
                .execute(state)
                .map_err(|source| RunError::Step { node: current, source })?;

            match transition {
                Transition::Goto { state: s, next } => {
                    self.check_edge(current, next)?;
                    state = s;
                    current = next;
                }
                Transition::Branch { state: s, mut next } => {
                    if next.is_empty() {
                        // Empty branch is treated as completion: the
                        // step said "fan out to nothing", so the run
                        // either resumes a queued branch or ends.
                        state = s;
                        match pending.pop() {
                            Some((resumed_state, resumed_node)) => {
                                state = resumed_state;
                                current = resumed_node;
                            }
                            None => {
                                return Ok(RunOutcome {
                                    final_state: state,
                                    reason: TerminalReason::Completed,
                                    trace,
                                });
                            }
                        }
                        continue;
                    }
                    for &n in &next {
                        self.check_edge(current, n)?;
                    }
                    let head = next.remove(0);
                    // Push remaining branches in reverse so the
                    // declared order is honoured when popping.
                    for n in next.into_iter().rev() {
                        pending.push((s.clone(), n));
                    }
                    state = s;
                    current = head;
                }
                Transition::Terminal { state: s, reason } => {
                    if pending.is_empty() {
                        return Ok(RunOutcome {
                            final_state: s,
                            reason,
                            trace,
                        });
                    }
                    // A terminal inside a branch terminates that branch
                    // but lets queued siblings continue. The outer run
                    // result reports the *first* terminal reason; later
                    // branches can only override by also terminating.
                    let (resumed_state, resumed_node) = pending.pop().unwrap();
                    let _ = (s, reason);
                    state = resumed_state;
                    current = resumed_node;
                }
            }
        }
    }

    fn check_edge(&self, from: NodeId, to: NodeId) -> Result<(), RunError<E>> {
        if self.graph.successors(from).any(|s| s == to) {
            Ok(())
        } else {
            Err(RunError::InvalidNextNode { node: to })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct State {
        counter: u32,
        log: Vec<&'static str>,
    }

    impl State {
        fn new() -> Self {
            Self { counter: 0, log: Vec::new() }
        }
    }

    /// Late-binding cell shared between the test body (which sets the
    /// id once it's known) and the step closure (which reads it at
    /// run time). Avoids the chicken-and-egg of "node A's body needs
    /// node B's id, but we don't have B's id until we add B".
    type LateId = Arc<OnceLock<NodeId>>;
    fn late() -> LateId {
        Arc::new(OnceLock::new())
    }
    fn lookup(cell: &LateId) -> NodeId {
        *cell.get().expect("late-bound id was never set")
    }

    #[test]
    fn linear_walk_runs_steps_in_order_and_terminates() {
        let mut o: Orchestrator<State, std::convert::Infallible> = Orchestrator::new();
        let a = o.add_step(FnStep::new("a", |mut s: State| {
            s.counter += 1;
            s.log.push("a");
            Ok(Transition::Terminal {
                state: s,
                reason: TerminalReason::Completed,
            })
        }));
        o.set_entry(a);

        let out = o.run(State::new(), StepLimits::default()).unwrap();
        assert_eq!(out.reason, TerminalReason::Completed);
        assert_eq!(out.final_state.counter, 1);
        assert_eq!(out.final_state.log, vec!["a"]);
        assert_eq!(out.trace, vec![a]);
    }

    #[test]
    fn goto_walks_through_three_nodes() {
        let mut o: Orchestrator<State, std::convert::Infallible> = Orchestrator::new();
        let next_b = late();
        let next_c = late();

        let a = {
            let next_b = next_b.clone();
            o.add_step(FnStep::new("a", move |mut s: State| {
                s.log.push("a");
                Ok(Transition::Goto { state: s, next: lookup(&next_b) })
            }))
        };
        let b = {
            let next_c = next_c.clone();
            o.add_step(FnStep::new("b", move |mut s: State| {
                s.log.push("b");
                Ok(Transition::Goto { state: s, next: lookup(&next_c) })
            }))
        };
        let c = o.add_step(FnStep::new("c", |mut s: State| {
            s.log.push("c");
            Ok(Transition::Terminal {
                state: s,
                reason: TerminalReason::Completed,
            })
        }));
        next_b.set(b).unwrap();
        next_c.set(c).unwrap();
        o.allow_transition(a, b).unwrap();
        o.allow_transition(b, c).unwrap();
        o.set_entry(a);

        let out = o.run(State::new(), StepLimits::default()).unwrap();
        assert_eq!(out.final_state.log, vec!["a", "b", "c"]);
        assert_eq!(out.trace, vec![a, b, c]);
        assert_eq!(out.reason, TerminalReason::Completed);
    }

    #[test]
    fn branch_visits_all_successors_in_declared_order() {
        let mut o: Orchestrator<State, std::convert::Infallible> = Orchestrator::new();
        let left = late();
        let right = late();

        let root = {
            let left = left.clone();
            let right = right.clone();
            o.add_step(FnStep::new("root", move |mut s: State| {
                s.log.push("root");
                Ok(Transition::Branch {
                    state: s,
                    next: vec![lookup(&left), lookup(&right)],
                })
            }))
        };
        let l = o.add_step(FnStep::new("left", |mut s: State| {
            s.log.push("left");
            Ok(Transition::Terminal {
                state: s,
                reason: TerminalReason::Completed,
            })
        }));
        let r = o.add_step(FnStep::new("right", |mut s: State| {
            s.log.push("right");
            Ok(Transition::Terminal {
                state: s,
                reason: TerminalReason::Completed,
            })
        }));
        left.set(l).unwrap();
        right.set(r).unwrap();
        o.allow_transition(root, l).unwrap();
        o.allow_transition(root, r).unwrap();
        o.set_entry(root);

        let out = o.run(State::new(), StepLimits::default()).unwrap();
        // root runs once, then both branches run; declared order is
        // left-first, then right is resumed from the pending stack.
        assert_eq!(out.trace, vec![root, l, r]);
        assert_eq!(out.reason, TerminalReason::Completed);
    }

    #[test]
    fn invalid_next_node_is_rejected() {
        let mut o: Orchestrator<u32, std::convert::Infallible> = Orchestrator::new();
        let stray = NodeId::new();
        let a = o.add_step(FnStep::new("a", move |s: u32| {
            Ok(Transition::Goto { state: s, next: stray })
        }));
        o.set_entry(a);
        let err = o.run(0, StepLimits::default()).unwrap_err();
        assert!(matches!(err, RunError::InvalidNextNode { .. }));
    }

    #[test]
    fn step_limit_guards_infinite_loops() {
        let mut o: Orchestrator<u32, std::convert::Infallible> = Orchestrator::new();
        let me = late();
        let a = {
            let me = me.clone();
            o.add_step(FnStep::new("loop", move |s: u32| {
                Ok(Transition::Goto { state: s + 1, next: lookup(&me) })
            }))
        };
        me.set(a).unwrap();
        o.allow_transition(a, a).unwrap();
        o.set_entry(a);
        let err = o
            .run(0, StepLimits { max_steps: Some(5) })
            .unwrap_err();
        assert!(matches!(err, RunError::StepLimitExceeded { limit: 5 }));
    }

    #[test]
    fn step_error_surfaces_with_node_context() {
        #[derive(Debug, PartialEq, Eq)]
        struct Boom;
        let mut o: Orchestrator<u32, Boom> = Orchestrator::new();
        let a = o.add_step(FnStep::new("err", |_s: u32| Err(Boom)));
        o.set_entry(a);
        let err = o.run(0, StepLimits::default()).unwrap_err();
        match err {
            RunError::Step { node, source } => {
                assert_eq!(node, a);
                assert_eq!(source, Boom);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn cancelled_terminal_propagates() {
        let mut o: Orchestrator<u32, std::convert::Infallible> = Orchestrator::new();
        let a = o.add_step(FnStep::new("cancel", |s: u32| {
            Ok(Transition::Terminal {
                state: s,
                reason: TerminalReason::Cancelled,
            })
        }));
        o.set_entry(a);
        let out = o.run(7, StepLimits::default()).unwrap();
        assert_eq!(out.reason, TerminalReason::Cancelled);
        assert_eq!(out.final_state, 7);
    }

    #[test]
    fn no_entry_is_an_error() {
        let o: Orchestrator<(), std::convert::Infallible> = Orchestrator::new();
        let err = o.run((), StepLimits::default()).unwrap_err();
        assert!(matches!(err, RunError::NoEntryNode));
    }
}
