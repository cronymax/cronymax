//! Generic directed-graph primitives used by [`crate::orchestration`].
//!
//! This module is deliberately minimal and **business-less**: it knows
//! nothing about agents, LLMs, capabilities, persistence, or hosts. It
//! exposes a small directed-graph data structure with stable node ids,
//! payload-typed nodes and edges, and the traversal helpers the
//! orchestration layer needs (successor lookup, topological order,
//! cycle detection).
//!
//! ## Shape
//!
//! `Graph<N, E>` stores nodes keyed by [`NodeId`] (a UUID newtype, so
//! ids are unique without consumers needing a counter) and adjacency as
//! `Vec<Edge<E>>` per source node. Iteration order over a single
//! source's outgoing edges is the insertion order, which keeps the
//! orchestration loop's branching deterministic.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Stable identifier for a node inside a [`Graph`]. Newtype over UUID
/// so callers never collide with each other and so ids survive
/// serialization round-trips.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub Uuid);

impl NodeId {
    /// Generate a fresh, never-before-seen id.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// A directed edge with a payload of type `E`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge<E> {
    pub target: NodeId,
    pub payload: E,
}

/// Directed graph generic over node payload `N` and edge payload `E`.
///
/// The graph never enforces acyclicity at insertion time —
/// orchestration may legitimately want loops (e.g. retry edges). Use
/// [`Graph::has_cycle`] or [`Graph::topological_order`] when a caller
/// explicitly wants the DAG invariant.
#[derive(Debug)]
pub struct Graph<N, E> {
    nodes: HashMap<NodeId, N>,
    /// Insertion-ordered adjacency. Preserved so iteration is
    /// deterministic for downstream consumers.
    adjacency: HashMap<NodeId, Vec<Edge<E>>>,
}

impl<N, E> Default for Graph<N, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<N, E> Graph<N, E> {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            adjacency: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Insert a node and return its newly-minted id.
    pub fn add_node(&mut self, payload: N) -> NodeId {
        let id = NodeId::new();
        self.nodes.insert(id, payload);
        self.adjacency.insert(id, Vec::new());
        id
    }

    /// Insert a node with a caller-chosen id. Returns the previous
    /// payload if one already existed for that id.
    pub fn insert_node_with_id(&mut self, id: NodeId, payload: N) -> Option<N> {
        self.adjacency.entry(id).or_default();
        self.nodes.insert(id, payload)
    }

    /// Add a directed edge `source -> target`. Returns
    /// [`GraphError::UnknownNode`] if either endpoint is missing.
    pub fn add_edge(
        &mut self,
        source: NodeId,
        target: NodeId,
        payload: E,
    ) -> Result<(), GraphError> {
        if !self.nodes.contains_key(&source) {
            return Err(GraphError::UnknownNode(source));
        }
        if !self.nodes.contains_key(&target) {
            return Err(GraphError::UnknownNode(target));
        }
        self.adjacency
            .entry(source)
            .or_default()
            .push(Edge { target, payload });
        Ok(())
    }

    pub fn node(&self, id: NodeId) -> Option<&N> {
        self.nodes.get(&id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut N> {
        self.nodes.get_mut(&id)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// Outgoing edges from `id`, in insertion order. Empty slice for
    /// missing or sink nodes.
    pub fn edges_from(&self, id: NodeId) -> &[Edge<E>] {
        self.adjacency.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Successor node ids in insertion order.
    pub fn successors(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.edges_from(id).iter().map(|e| e.target)
    }

    /// All node ids. Order is unspecified (HashMap iteration).
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes.keys().copied()
    }

    /// True iff the graph contains at least one directed cycle.
    pub fn has_cycle(&self) -> bool {
        #[derive(Clone, Copy, PartialEq)]
        enum Color {
            White,
            Grey,
            Black,
        }
        let mut color: HashMap<NodeId, Color> =
            self.nodes.keys().map(|&id| (id, Color::White)).collect();

        // Iterative DFS to avoid stack overflow on deep graphs.
        for &start in self.nodes.keys() {
            if color[&start] != Color::White {
                continue;
            }
            let mut stack: Vec<(NodeId, usize)> = vec![(start, 0)];
            color.insert(start, Color::Grey);
            while let Some(&(node, edge_idx)) = stack.last() {
                let edges = self.edges_from(node);
                if edge_idx >= edges.len() {
                    color.insert(node, Color::Black);
                    stack.pop();
                    continue;
                }
                stack.last_mut().unwrap().1 = edge_idx + 1;
                let next = edges[edge_idx].target;
                match color.get(&next).copied().unwrap_or(Color::White) {
                    Color::Grey => return true,
                    Color::White => {
                        color.insert(next, Color::Grey);
                        stack.push((next, 0));
                    }
                    Color::Black => {}
                }
            }
        }
        false
    }

    /// Kahn's algorithm. Returns nodes in a valid topological order or
    /// [`GraphError::Cycle`] if the graph is cyclic.
    pub fn topological_order(&self) -> Result<Vec<NodeId>, GraphError> {
        let mut indegree: HashMap<NodeId, usize> = self.nodes.keys().map(|&id| (id, 0)).collect();
        for edges in self.adjacency.values() {
            for edge in edges {
                *indegree.entry(edge.target).or_insert(0) += 1;
            }
        }
        let mut ready: VecDeque<NodeId> = indegree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&id, _)| id)
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut visited: HashSet<NodeId> = HashSet::new();
        while let Some(id) = ready.pop_front() {
            if !visited.insert(id) {
                continue;
            }
            order.push(id);
            for edge in self.edges_from(id) {
                let entry = indegree.get_mut(&edge.target).unwrap();
                *entry -= 1;
                if *entry == 0 {
                    ready.push_back(edge.target);
                }
            }
        }
        if order.len() != self.nodes.len() {
            Err(GraphError::Cycle)
        } else {
            Ok(order)
        }
    }
}

/// Errors surfaced by [`Graph`] operations and traversal helpers.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GraphError {
    #[error("unknown node id: {0}")]
    UnknownNode(NodeId),
    #[error("graph contains a cycle")]
    Cycle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_query_nodes_and_edges() {
        let mut g: Graph<&str, ()> = Graph::new();
        let a = g.add_node("a");
        let b = g.add_node("b");
        let c = g.add_node("c");
        g.add_edge(a, b, ()).unwrap();
        g.add_edge(a, c, ()).unwrap();
        g.add_edge(b, c, ()).unwrap();

        assert_eq!(g.len(), 3);
        assert_eq!(g.node(a), Some(&"a"));
        let succs: Vec<_> = g.successors(a).collect();
        assert_eq!(succs, vec![b, c], "insertion order preserved");
    }

    #[test]
    fn unknown_node_rejected() {
        let mut g: Graph<(), ()> = Graph::new();
        let a = g.add_node(());
        let phantom = NodeId::new();
        assert_eq!(
            g.add_edge(a, phantom, ()),
            Err(GraphError::UnknownNode(phantom))
        );
        assert_eq!(
            g.add_edge(phantom, a, ()),
            Err(GraphError::UnknownNode(phantom))
        );
    }

    #[test]
    fn cycle_detection_and_topo_sort() {
        let mut g: Graph<u32, ()> = Graph::new();
        let a = g.add_node(1);
        let b = g.add_node(2);
        let c = g.add_node(3);
        g.add_edge(a, b, ()).unwrap();
        g.add_edge(b, c, ()).unwrap();
        assert!(!g.has_cycle());
        let order = g.topological_order().unwrap();
        assert_eq!(order.len(), 3);
        let pos = |x: NodeId| order.iter().position(|&y| y == x).unwrap();
        assert!(pos(a) < pos(b));
        assert!(pos(b) < pos(c));

        g.add_edge(c, a, ()).unwrap();
        assert!(g.has_cycle());
        assert_eq!(g.topological_order(), Err(GraphError::Cycle));
    }
}
