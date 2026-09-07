//! Deterministic graph analysis over the board's semantic IDs. No second store
//! or scheduler: dependency order is structural, while runtime readiness remains
//! the workflow engine's decision. Iterative walks tolerate deep agent plans.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub type Adjacency = BTreeMap<String, BTreeSet<String>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DagAnalysis {
    /// Prerequisites first. Members of a layer can be built independently.
    pub layers: Vec<Vec<String>>,
    /// Actual cyclic components, not innocent descendants blocked behind them.
    pub cycles: Vec<Vec<String>>,
    /// No build order exists for these nodes (cycles, missing targets or their
    /// dependents). Never silently emit them as roots.
    pub unbuildable: Vec<String>,
}

/// Shortest witness from a proposed dependency to the edited task. Unrelated
/// cycles cannot mask this one. Parents reconstruct a path without cloning an
/// entire path on every edge of a large plan.
pub fn path_to(graph: &Adjacency, starts: &[String], target: &str) -> Option<Vec<String>> {
    let mut queue = VecDeque::new();
    let mut parents = BTreeMap::<String, Option<String>>::new();
    for start in starts {
        if !parents.contains_key(start) {
            parents.insert(start.clone(), None);
            queue.push_back(start.clone());
        }
    }
    while let Some(node) = queue.pop_front() {
        if node == target {
            let mut path = vec![node];
            while let Some(Some(parent)) = parents.get(path.last().unwrap()) {
                path.push(parent.clone());
            }
            path.reverse();
            return Some(path);
        }
        for next in graph.get(&node).into_iter().flatten() {
            if !parents.contains_key(next) {
                parents.insert(next.clone(), Some(node.clone()));
                queue.push_back(next.clone());
            }
        }
    }
    None
}

pub fn analyze(nodes: &BTreeSet<String>, graph: &Adjacency) -> DagAnalysis {
    let mut pending = BTreeMap::new();
    let mut reverse: Adjacency = BTreeMap::new();
    for node in nodes {
        let deps = graph.get(node);
        pending.insert(node.clone(), deps.map_or(0, BTreeSet::len));
        for dep in deps.into_iter().flatten() {
            reverse.entry(dep.clone()).or_default().insert(node.clone());
        }
    }
    let mut ready: BTreeSet<String> = pending
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut layers = Vec::new();
    while !ready.is_empty() {
        let layer: Vec<String> = ready.into_iter().collect();
        ready = BTreeSet::new();
        for node in &layer {
            pending.remove(node);
            for dependent in reverse.get(node).into_iter().flatten() {
                if let Some(n) = pending.get_mut(dependent) {
                    *n -= 1;
                    if *n == 0 {
                        ready.insert(dependent.clone());
                    }
                }
            }
        }
        layers.push(layer);
    }

    // Kosaraju, both passes iterative: O(V+E) apart from ordered-map costs.
    // Only the residue can contain a cycle; missing targets remain absent.
    let residue: BTreeSet<String> = pending.keys().cloned().collect();
    let mut seen = BTreeSet::new();
    let mut finish = Vec::new();
    for root in &residue {
        let mut stack = vec![(root.clone(), false)];
        while let Some((node, exiting)) = stack.pop() {
            if exiting {
                finish.push(node);
                continue;
            }
            if !seen.insert(node.clone()) {
                continue;
            }
            stack.push((node.clone(), true));
            for next in graph.get(&node).into_iter().flatten().rev() {
                if residue.contains(next) && !seen.contains(next) {
                    stack.push((next.clone(), false));
                }
            }
        }
    }
    seen.clear();
    let mut cycles = Vec::new();
    for root in finish.into_iter().rev() {
        if seen.contains(&root) {
            continue;
        }
        let mut component = BTreeSet::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if !seen.insert(node.clone()) {
                continue;
            }
            component.insert(node.clone());
            for next in reverse.get(&node).into_iter().flatten() {
                if residue.contains(next) && !seen.contains(next) {
                    stack.push(next.clone());
                }
            }
        }
        let self_cycle = component
            .first()
            .is_some_and(|n| graph.get(n).is_some_and(|deps| deps.contains(n)));
        if component.len() > 1 || self_cycle {
            cycles.push(component.into_iter().collect());
        }
    }
    cycles.sort();
    DagAnalysis {
        layers,
        cycles,
        unbuildable: residue.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn graph(edges: &[(&str, &str)]) -> Adjacency {
        let mut graph = Adjacency::new();
        for (a, b) in edges {
            graph
                .entry(a.to_string())
                .or_default()
                .insert(b.to_string());
        }
        graph
    }
    #[test]
    fn diamonds_build_in_stable_parallel_layers() {
        let g = graph(&[("D", "B"), ("D", "C"), ("B", "A"), ("C", "A")]);
        let result = analyze(&["A", "B", "C", "D"].map(String::from).into(), &g);
        assert_eq!(result.layers, vec![vec!["A"], vec!["B", "C"], vec!["D"]]);
        assert!(result.cycles.is_empty());
        assert!(result.unbuildable.is_empty());
    }
    #[test]
    fn all_cycles_are_found_without_accusing_their_descendants() {
        let g = graph(&[
            ("A", "B"),
            ("B", "A"),
            ("C", "D"),
            ("D", "C"),
            ("E", "D"),
            ("F", "missing"),
        ]);
        let result = analyze(
            &["A", "B", "C", "D", "E", "F", "G"].map(String::from).into(),
            &g,
        );
        assert_eq!(result.cycles, vec![vec!["A", "B"], vec!["C", "D"]]);
        assert_eq!(result.layers, vec![vec!["G"]]);
        assert_eq!(result.unbuildable, vec!["A", "B", "C", "D", "E", "F"]);
        assert_eq!(
            path_to(&g, &["C".into()], "D"),
            Some(vec!["C".into(), "D".into()])
        );
    }
    #[test]
    fn deep_graphs_do_not_use_the_call_stack() {
        let mut g = Adjacency::new();
        for n in 0..10000 {
            g.insert(n.to_string(), [(n + 1).to_string()].into());
        }
        let nodes = (0..=10000).map(|n| n.to_string()).collect();
        assert_eq!(analyze(&nodes, &g).layers.len(), 10001);
        g.insert("10000".into(), ["0".into()].into());
        assert_eq!(analyze(&nodes, &g).cycles[0].len(), 10001);
    }
}
