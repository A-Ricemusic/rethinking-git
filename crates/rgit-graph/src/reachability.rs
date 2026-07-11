use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
};

use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphNode<K> {
    pub id: K,
    pub parents: Vec<K>,
}

impl<K> GraphNode<K> {
    #[must_use]
    pub const fn new(id: K, parents: Vec<K>) -> Self {
        Self { id, parents }
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum GenerationError {
    #[error("graph generation overflow")]
    Overflow,
}

/// Calculates a node generation as zero for a root, otherwise one greater
/// than the maximum parent generation.
pub fn next_generation(
    parent_generations: impl IntoIterator<Item = u64>,
) -> Result<u64, GenerationError> {
    match parent_generations.into_iter().max() {
        None => Ok(0),
        Some(generation) => generation.checked_add(1).ok_or(GenerationError::Overflow),
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GraphError<K: Debug> {
    #[error("graph contains duplicate node {node:?}")]
    DuplicateNode { node: K },
    #[error("node {node:?} refers to missing parent {parent:?}")]
    MissingParent { node: K, parent: K },
    #[error("graph contains a cycle involving nodes {cycle_nodes:?}")]
    Cycle { cycle_nodes: Vec<K> },
    #[error("generation overflow while processing node {node:?}")]
    GenerationOverflow { node: K },
    #[error("node {node:?} is not present in the graph")]
    UnknownNode { node: K },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IndexedNode<K> {
    parents: Vec<K>,
    generation: u64,
}

/// A validated DAG with deterministic generations and reachability queries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphIndex<K> {
    nodes: BTreeMap<K, IndexedNode<K>>,
    topological_order: Vec<K>,
}

impl<K> GraphIndex<K>
where
    K: Clone + Debug + Ord,
{
    pub fn build(nodes: impl IntoIterator<Item = GraphNode<K>>) -> Result<Self, GraphError<K>> {
        let mut parents_by_node = BTreeMap::new();
        for node in nodes {
            let id = node.id;
            let parents = node.parents.into_iter().collect::<BTreeSet<_>>();
            if parents_by_node.insert(id.clone(), parents).is_some() {
                return Err(GraphError::DuplicateNode { node: id });
            }
        }

        for (node, parents) in &parents_by_node {
            for parent in parents {
                if !parents_by_node.contains_key(parent) {
                    return Err(GraphError::MissingParent {
                        node: node.clone(),
                        parent: parent.clone(),
                    });
                }
            }
        }

        let mut remaining_parents = parents_by_node
            .iter()
            .map(|(node, parents)| (node.clone(), parents.len()))
            .collect::<BTreeMap<_, _>>();
        let mut children = parents_by_node
            .keys()
            .cloned()
            .map(|node| (node, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        for (node, parents) in &parents_by_node {
            for parent in parents {
                children
                    .get_mut(parent)
                    .expect("parent existence was validated")
                    .insert(node.clone());
            }
        }

        let mut ready = remaining_parents
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(node, _)| node.clone())
            .collect::<BTreeSet<_>>();
        let mut generations = BTreeMap::new();
        let mut topological_order = Vec::with_capacity(parents_by_node.len());
        while let Some(node) = ready.pop_first() {
            let generation = next_generation(
                parents_by_node[&node]
                    .iter()
                    .map(|parent| generations[parent]),
            )
            .map_err(|GenerationError::Overflow| GraphError::GenerationOverflow {
                node: node.clone(),
            })?;
            generations.insert(node.clone(), generation);
            topological_order.push(node.clone());

            for child in &children[&node] {
                let count = remaining_parents
                    .get_mut(child)
                    .expect("every child is a graph node");
                *count -= 1;
                if *count == 0 {
                    ready.insert(child.clone());
                }
            }
        }

        if topological_order.len() != parents_by_node.len() {
            let blocked_nodes = remaining_parents
                .into_iter()
                .filter(|(_, count)| *count != 0)
                .map(|(node, _)| node)
                .collect();
            return Err(GraphError::Cycle {
                cycle_nodes: cycle_members(&parents_by_node, &blocked_nodes),
            });
        }

        let nodes = parents_by_node
            .into_iter()
            .map(|(node, parents)| {
                let indexed = IndexedNode {
                    parents: parents.into_iter().collect(),
                    generation: generations[&node],
                };
                (node, indexed)
            })
            .collect();
        Ok(Self {
            nodes,
            topological_order,
        })
    }

    pub fn generation(&self, node: &K) -> Result<u64, GraphError<K>> {
        self.nodes
            .get(node)
            .map(|node| node.generation)
            .ok_or_else(|| GraphError::UnknownNode { node: node.clone() })
    }

    pub fn ancestors(&self, node: &K) -> Result<BTreeSet<K>, GraphError<K>> {
        if !self.nodes.contains_key(node) {
            return Err(GraphError::UnknownNode { node: node.clone() });
        }
        let mut ancestors = BTreeSet::new();
        let mut pending = self.nodes[node].parents.clone();
        while let Some(parent) = pending.pop() {
            if ancestors.insert(parent.clone()) {
                pending.extend(self.nodes[&parent].parents.iter().cloned());
            }
        }
        Ok(ancestors)
    }

    /// Returns true when `ancestor == descendant` or an ancestry path exists.
    pub fn is_reachable(&self, ancestor: &K, descendant: &K) -> Result<bool, GraphError<K>> {
        if !self.nodes.contains_key(ancestor) {
            return Err(GraphError::UnknownNode {
                node: ancestor.clone(),
            });
        }
        if ancestor == descendant {
            return Ok(true);
        }
        Ok(self.ancestors(descendant)?.contains(ancestor))
    }

    #[must_use]
    pub fn topological_order(&self) -> &[K] {
        &self.topological_order
    }
}

fn cycle_members<K>(
    parents_by_node: &BTreeMap<K, BTreeSet<K>>,
    blocked_nodes: &BTreeSet<K>,
) -> Vec<K>
where
    K: Clone + Ord,
{
    // A blocked node is in a cycle exactly when one of its blocked parents can
    // reach it. This deliberately excludes acyclic descendants merely blocked
    // behind a cycle. Iterative traversal avoids graph-size-dependent stack
    // use on untrusted repository metadata.
    blocked_nodes
        .iter()
        .filter(|node| {
            let mut pending = parents_by_node[*node]
                .iter()
                .filter(|parent| blocked_nodes.contains(*parent))
                .cloned()
                .collect::<Vec<_>>();
            let mut visited = BTreeSet::new();
            while let Some(candidate) = pending.pop() {
                if &candidate == *node {
                    return true;
                }
                if visited.insert(candidate.clone()) {
                    pending.extend(
                        parents_by_node[&candidate]
                            .iter()
                            .filter(|parent| blocked_nodes.contains(*parent))
                            .cloned(),
                    );
                }
            }
            false
        })
        .cloned()
        .collect()
}
