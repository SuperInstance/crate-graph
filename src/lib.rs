use std::collections::{HashMap, HashSet, BTreeSet};

#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;

/// A Rust crate with its metadata parsed from Cargo.toml.
#[derive(Debug, Clone)]
pub struct CrateNode {
    pub name: String,
    pub version: String,
    pub dependencies: Vec<String>,
    pub features: Vec<String>,
    pub path: String,
}

impl CrateNode {
    /// Parse a Cargo.toml string into a CrateNode.
    pub fn from_toml_str(content: &str, path: &str) -> Result<Self> {
        let value: toml::Value = content.parse().context("failed to parse Cargo.toml")?;

        let package = value.get("package").context("missing [package]")?;
        let name = package
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let version = package
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.0.0")
            .to_string();

        let features: Vec<String> = value
            .get("features")
            .and_then(|v| v.as_table())
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default();

        let mut deps = Vec::new();

        // [dependencies]
        if let Some(dep_table) = value.get("dependencies").and_then(|v| v.as_table()) {
            for key in dep_table.keys() {
                deps.push(key.clone());
            }
        }

        // [dev-dependencies]
        if let Some(dep_table) = value.get("dev-dependencies").and_then(|v| v.as_table()) {
            for key in dep_table.keys() {
                deps.push(key.clone());
            }
        }

        Ok(CrateNode {
            name,
            version,
            dependencies: deps,
            features,
            path: path.to_string(),
        })
    }
}

/// A directed acyclic graph of crate dependencies built from a fleet of Cargo.toml files.
#[derive(Debug)]
pub struct DependencyGraph {
    graph: DiGraph<CrateNode, ()>,
    name_index: HashMap<String, NodeIndex>,
}

impl DependencyGraph {
    /// Build a graph from a slice of CrateNodes.
    /// Returns an error if cycles are detected.
    pub fn build(crates: Vec<CrateNode>) -> Result<Self> {
        let mut graph = DiGraph::new();
        let mut name_index = HashMap::new();

        // Add all nodes first.
        for krate in &crates {
            let idx = graph.add_node(krate.clone());
            name_index.insert(krate.name.clone(), idx);
        }

        // Add edges: dependency -> dependent (dep points to crate that depends on it).
        // We model edges as: from dependency to dependent.
        for krate in &crates {
            let dep_idx = *name_index.get(&krate.name).context("missing node")?;
            for dep_name in &krate.dependencies {
                if let Some(&src_idx) = name_index.get(dep_name) {
                    // Edge: src (dependency) -> dep_idx (dependent crate)
                    graph.add_edge(src_idx, dep_idx, ());
                }
                // Skip deps not in our fleet (external crates).
            }
        }

        // Check for cycles.
        let sorted = toposort(&graph, None);
        if let Err(cycle) = sorted {
            let node = &graph[cycle.node_id()];
            anyhow::bail!(
                "cycle detected involving crate: {} (at {})",
                node.name,
                node.path
            );
        }

        Ok(DependencyGraph {
            graph,
            name_index,
        })
    }

    /// Build allowing cycles (returns graph + cycle info instead of erroring).
    pub fn build_allow_cycles(crates: Vec<CrateNode>) -> Result<(Self, Vec<String>)> {
        let mut graph = DiGraph::new();
        let mut name_index = HashMap::new();

        for krate in &crates {
            let idx = graph.add_node(krate.clone());
            name_index.insert(krate.name.clone(), idx);
        }

        for krate in &crates {
            let dep_idx = *name_index.get(&krate.name).context("missing node")?;
            for dep_name in &krate.dependencies {
                if let Some(&src_idx) = name_index.get(dep_name) {
                    graph.add_edge(src_idx, dep_idx, ());
                }
            }
        }

        let mut cycle_nodes = Vec::new();
        let sorted = toposort(&graph, None);
        if let Err(cycle) = sorted {
            cycle_nodes.push(graph[cycle.node_id()].name.clone());
        }

        Ok((
            DependencyGraph { graph, name_index },
            cycle_nodes,
        ))
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    pub fn crates(&self) -> Vec<&CrateNode> {
        self.graph.node_indices().map(|i| &self.graph[i]).collect()
    }

    pub fn get_node(&self, name: &str) -> Option<&CrateNode> {
        self.name_index.get(name).map(|&idx| &self.graph[idx])
    }

    pub fn contains(&self, name: &str) -> bool {
        self.name_index.contains_key(name)
    }

    /// Get all crates that directly depend on the given crate.
    pub fn direct_dependents(&self, name: &str) -> Vec<&CrateNode> {
        if let Some(&idx) = self.name_index.get(name) {
            self.graph
                .neighbors_directed(idx, Direction::Outgoing)
                .map(|i| &self.graph[i])
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get all crates the given crate directly depends on.
    pub fn direct_dependencies(&self, name: &str) -> Vec<&CrateNode> {
        if let Some(&idx) = self.name_index.get(name) {
            self.graph
                .neighbors_directed(idx, Direction::Incoming)
                .map(|i| &self.graph[i])
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn graph(&self) -> &DiGraph<CrateNode, ()> {
        &self.graph
    }

    pub fn name_index(&self) -> &HashMap<String, NodeIndex> {
        &self.name_index
    }
}

/// Computes dependency layers.
/// Layer 0 = crates with no in-fleet dependencies.
/// Layer N = depends only on crates in layers 0..N.
pub struct LayerCalculator;

impl LayerCalculator {
    /// Compute layers for all crates in the graph.
    /// Returns a map from layer number to crate names in that layer.
    pub fn compute(graph: &DependencyGraph) -> Vec<BTreeSet<String>> {
        let g = graph.graph();
        let mut layers: Vec<BTreeSet<String>> = Vec::new();
        let mut assigned: HashMap<NodeIndex, usize> = HashMap::new();

        // Iteratively assign layers.
        let total = g.node_count();
        while assigned.len() < total {
            let mut current_layer = BTreeSet::new();
            let mut newly_assigned: Vec<NodeIndex> = Vec::new();
            for idx in g.node_indices() {
                if assigned.contains_key(&idx) {
                    continue;
                }
                // Check all incoming neighbors (dependencies of this crate) are assigned.
                let deps_assigned = g
                    .neighbors_directed(idx, Direction::Incoming)
                    .all(|dep| assigned.contains_key(&dep));
                if deps_assigned {
                    current_layer.insert(g[idx].name.clone());
                    newly_assigned.push(idx);
                }
            }
            if current_layer.is_empty() {
                break;
            }
            let layer_num = layers.len();
            for idx in newly_assigned {
                assigned.insert(idx, layer_num);
            }
            layers.push(current_layer);
        }

        layers
    }

    /// Get the layer number for a specific crate.
    pub fn layer_of(graph: &DependencyGraph, name: &str) -> Option<usize> {
        let layers = Self::compute(graph);
        for (i, layer) in layers.iter().enumerate() {
            if layer.contains(name) {
                return Some(i);
            }
        }
        None
    }
}

/// Finds the longest dependency chain in the fleet (critical path).
pub struct CriticalPath;

impl CriticalPath {
    /// Find the longest path in the DAG.
    /// Returns (length, path of crate names).
    /// Handles cyclic graphs by treating them as DAG with cycle-breaking.
    pub fn find(graph: &DependencyGraph) -> (usize, Vec<String>) {
        let g = graph.graph();

        let sorted = toposort(g, None).unwrap_or_else(|e| {
            // For cyclic graphs, do a manual topo sort excluding cycle edges
            let mut result = Vec::new();
            let mut visited = HashSet::new();
            let mut on_stack = HashSet::new();
            for idx in g.node_indices() {
                Self::dfs_topo(g, idx, &mut visited, &mut on_stack, &mut result);
            }
            result
        });

        let mut best_len = 0usize;
        let mut best_path: Vec<String> = Vec::new();
        let mut dist: HashMap<NodeIndex, usize> = HashMap::new();
        let mut predecessor: HashMap<NodeIndex, Option<NodeIndex>> = HashMap::new();

        for &node in &sorted {
            let mut max_pred_dist = 0usize;
            let mut max_pred: Option<NodeIndex> = None;

            for pred in g.neighbors_directed(node, Direction::Incoming) {
                if let Some(&d) = dist.get(&pred) {
                    if d + 1 > max_pred_dist {
                        max_pred_dist = d + 1;
                        max_pred = Some(pred);
                    }
                }
            }

            dist.insert(node, max_pred_dist);
            predecessor.insert(node, max_pred);

            if max_pred_dist > best_len {
                best_len = max_pred_dist;
                best_path = Vec::new();
                let mut cur = Some(node);
                while let Some(n) = cur {
                    best_path.push(g[n].name.clone());
                    cur = predecessor.get(&n).copied().flatten();
                }
                best_path.reverse();
            }
        }

        if best_path.is_empty() && !sorted.is_empty() {
            best_path.push(g[sorted[0]].name.clone());
        }

        (best_len, best_path)
    }

    fn dfs_topo(
        g: &DiGraph<CrateNode, ()>,
        node: NodeIndex,
        visited: &mut HashSet<NodeIndex>,
        on_stack: &mut HashSet<NodeIndex>,
        result: &mut Vec<NodeIndex>,
    ) {
        if visited.contains(&node) {
            return;
        }
        if on_stack.contains(&node) {
            return; // cycle — skip
        }
        on_stack.insert(node);
        for neighbor in g.neighbors_directed(node, Direction::Incoming) {
            Self::dfs_topo(g, neighbor, visited, on_stack, result);
        }
        on_stack.remove(&node);
        visited.insert(node);
        result.push(node);
    }

    /// Find the longest chain that includes a specific crate.
    pub fn find_through(graph: &DependencyGraph, name: &str) -> (usize, Vec<String>) {
        let g = graph.graph();
        let ni = graph.name_index();
        let start = match ni.get(name) {
            Some(i) => *i,
            None => return (0, Vec::new()),
        };

        // Find longest path to this node from roots.
        let sorted = toposort(g, None).unwrap_or_default();
        let mut dist_to: HashMap<NodeIndex, usize> = HashMap::new();
        let mut pred_to: HashMap<NodeIndex, Option<NodeIndex>> = HashMap::new();

        for &node in &sorted {
            let mut max_d = 0usize;
            let mut max_p: Option<NodeIndex> = None;
            for p in g.neighbors_directed(node, Direction::Incoming) {
                if let Some(&d) = dist_to.get(&p) {
                    if d + 1 > max_d {
                        max_d = d + 1;
                        max_p = Some(p);
                    }
                }
            }
            dist_to.insert(node, max_d);
            pred_to.insert(node, max_p);
        }

        // Find longest path from this node to leaves.
        let mut dist_from: HashMap<NodeIndex, usize> = HashMap::new();
        let mut succ_from: HashMap<NodeIndex, Option<NodeIndex>> = HashMap::new();

        for &node in sorted.iter().rev() {
            let mut max_d = 0usize;
            let mut max_s: Option<NodeIndex> = None;
            for s in g.neighbors_directed(node, Direction::Outgoing) {
                if let Some(&d) = dist_from.get(&s) {
                    if d + 1 > max_d {
                        max_d = d + 1;
                        max_s = Some(s);
                    }
                }
            }
            dist_from.insert(node, max_d);
            succ_from.insert(node, max_s);
        }

        let total_len = dist_to.get(&start).copied().unwrap_or(0)
            + dist_from.get(&start).copied().unwrap_or(0);

        // Reconstruct path: root -> ... -> start -> ... -> leaf
        let mut path = Vec::new();
        // Backtrack to root.
        let mut chain_to = Vec::new();
        let mut cur = Some(start);
        while let Some(n) = cur {
            chain_to.push(g[n].name.clone());
            cur = pred_to.get(&n).copied().flatten();
        }
        chain_to.reverse();

        // Forward to leaf.
        let mut chain_from = Vec::new();
        cur = succ_from.get(&start).copied().flatten();
        while let Some(n) = cur {
            chain_from.push(g[n].name.clone());
            cur = succ_from.get(&n).copied().flatten();
        }

        path = chain_to;
        path.extend(chain_from);

        (total_len, path)
    }
}

/// Analyzes the blast radius of a change to a crate.
pub struct ImpactAnalyzer<'a> {
    graph: &'a DependencyGraph,
}

impl<'a> ImpactAnalyzer<'a> {
    pub fn new(graph: &'a DependencyGraph) -> Self {
        ImpactAnalyzer { graph }
    }

    /// Count all downstream crates (direct + transitive dependents).
    pub fn downstream_count(&self, name: &str) -> usize {
        self.downstream_set(name).len()
    }

    /// Get the set of all downstream crate names.
    pub fn downstream_set(&self, name: &str) -> HashSet<String> {
        let g = self.graph.graph();
        let ni = self.graph.name_index();
        let mut visited = HashSet::new();

        if let Some(&start) = ni.get(name) {
            let mut stack = vec![start];
            while let Some(node) = stack.pop() {
                for neighbor in g.neighbors_directed(node, Direction::Outgoing) {
                    let nname = &g[neighbor].name;
                    if visited.insert(nname.clone()) {
                        stack.push(neighbor);
                    }
                }
            }
        }

        visited
    }

    /// Get downstream crates organized by distance (how many hops away).
    pub fn downstream_by_depth(&self, name: &str) -> Vec<BTreeSet<String>> {
        let g = self.graph.graph();
        let ni = self.graph.name_index();
        let mut layers: Vec<BTreeSet<String>> = Vec::new();
        let mut visited: HashSet<NodeIndex> = HashSet::new();

        if let Some(&start) = ni.get(name) {
            let mut current = vec![start];
            visited.insert(start);

            while !current.is_empty() {
                let mut next = Vec::new();
                let mut layer = BTreeSet::new();

                for node in &current {
                    for neighbor in g.neighbors_directed(*node, Direction::Outgoing) {
                        if visited.insert(neighbor) {
                            layer.insert(g[neighbor].name.clone());
                            next.push(neighbor);
                        }
                    }
                }

                if !layer.is_empty() {
                    layers.push(layer);
                }
                current = next;
            }
        }

        layers
    }

    /// Find the most-depended-on crate in the fleet.
    pub fn most_depended_on(&self) -> (String, usize) {
        let g = self.graph.graph();
        let mut best_name = String::new();
        let mut best_count = 0usize;

        for idx in g.node_indices() {
            let count = self.downstream_count(&g[idx].name);
            if count > best_count {
                best_count = count;
                best_name = g[idx].name.clone();
            }
        }

        (best_name, best_count)
    }

    /// Rank all crates by downstream impact.
    pub fn rank_by_impact(&self) -> Vec<(String, usize)> {
        let g = self.graph.graph();
        let mut ranks: Vec<(String, usize)> = g
            .node_indices()
            .map(|idx| {
                let name = g[idx].name.clone();
                let count = self.downstream_count(&name);
                (name, count)
            })
            .collect();
        ranks.sort_by(|a, b| b.1.cmp(&a.1));
        ranks
    }
}

/// Checks if two dependency subgraphs are structurally identical (bisimulation).
/// A borrowed concept from category theory / concurrency theory.
pub struct BisimulationChecker;

impl BisimulationChecker {
    /// Check if two crate subgraphs rooted at `name_a` and `name_b` are bisimilar.
    /// Two subgraphs are bisimilar if there exists a relation R such that:
    /// - (a, b) in R implies their direct dependency sets are matched via R
    /// - For every dependency a' of a, there exists b' of b such that (a', b') in R
    ///   and vice versa.
    pub fn check(
        graph: &DependencyGraph,
        name_a: &str,
        name_b: &str,
    ) -> bool {
        let g = graph.graph();
        let ni = graph.name_index();

        let a = match ni.get(name_a) {
            Some(&i) => i,
            None => return false,
        };
        let b = match ni.get(name_b) {
            Some(&i) => i,
            None => return false,
        };

        // Check basic structural equivalence.
        let a_deps: BTreeSet<NodeIndex> = g
            .neighbors_directed(a, Direction::Incoming)
            .collect();
        let b_deps: BTreeSet<NodeIndex> = g
            .neighbors_directed(b, Direction::Incoming)
            .collect();

        if a_deps.len() != b_deps.len() {
            return false;
        }

        // Build a bisimulation relation iteratively.
        let mut relation: HashSet<(NodeIndex, NodeIndex)> = HashSet::new();
        let mut worklist = vec![(a, b)];

        while let Some((x, y)) = worklist.pop() {
            if relation.contains(&(x, y)) {
                continue;
            }

            let x_deps: Vec<NodeIndex> = g
                .neighbors_directed(x, Direction::Incoming)
                .collect();
            let y_deps: Vec<NodeIndex> = g
                .neighbors_directed(y, Direction::Incoming)
                .collect();

            if x_deps.len() != y_deps.len() {
                return false;
            }

            relation.insert((x, y));

            // Try to find a matching for each x_dep to a y_dep.
            // Use greedy matching with bisimulation check.
            let mut y_matched: HashSet<NodeIndex> = HashSet::new();

            for &xd in &x_deps {
                let mut found = false;
                for &yd in &y_deps {
                    if y_matched.contains(&yd) {
                        continue;
                    }
                    // Check structural compatibility at this level.
                    let xd_deps = g
                        .neighbors_directed(xd, Direction::Incoming)
                        .count();
                    let yd_deps = g
                        .neighbors_directed(yd, Direction::Incoming)
                        .count();
                    if xd_deps == yd_deps {
                        y_matched.insert(yd);
                        worklist.push((xd, yd));
                        found = true;
                        break;
                    }
                }
                if !found {
                    return false;
                }
            }
        }

        // Verify the relation is a valid bisimulation.
        for &(x, y) in &relation {
            let x_deps: Vec<NodeIndex> = g
                .neighbors_directed(x, Direction::Incoming)
                .collect();
            let y_deps: Vec<NodeIndex> = g
                .neighbors_directed(y, Direction::Incoming)
                .collect();

            // For each x_dep, must have matching y_dep in relation.
            for xd in &x_deps {
                let has_match = relation.iter().any(|(a, b)| *a == *xd && y_deps.contains(b));
                if !has_match {
                    // Also accept if x_dep is not in our fleet (external).
                    if ni.contains_key(&g[*xd].name) {
                        return false;
                    }
                }
            }

            for yd in &y_deps {
                let has_match = relation.iter().any(|(a, b)| *b == *yd && x_deps.contains(a));
                if !has_match {
                    if ni.contains_key(&g[*yd].name) {
                        return false;
                    }
                }
            }
        }

        true
    }

    /// Find all pairs of crates that have structurally identical dependency subgraphs.
    pub fn find_bisimilar_pairs(graph: &DependencyGraph) -> Vec<(String, String)> {
        let crates: Vec<&CrateNode> = graph.crates();
        let mut pairs = Vec::new();

        // Group by dependency count for efficiency.
        let mut by_dep_count: HashMap<usize, Vec<&str>> = HashMap::new();
        for krate in &crates {
            let count = graph.direct_dependencies(&krate.name).len();
            by_dep_count.entry(count).or_default().push(&krate.name);
        }

        for (_, group) in &by_dep_count {
            for i in 0..group.len() {
                for j in (i + 1)..group.len() {
                    if Self::check(graph, group[i], group[j]) {
                        pairs.push((group[i].to_string(), group[j].to_string()));
                    }
                }
            }
        }

        pairs
    }
}

/// Scan a directory for Cargo.toml files and parse them.
pub fn scan_cargo_tomls(dir: &str) -> Result<Vec<CrateNode>> {
    let mut crates = Vec::new();
    scan_recursive(dir, &mut crates)?;
    Ok(crates)
}

fn scan_recursive(dir: &str, crates: &mut Vec<CrateNode>) -> Result<()> {
    let entries = std::fs::read_dir(dir).context("cannot read dir")?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            let cargo = path.join("Cargo.toml");
            if cargo.exists() {
                let content = std::fs::read_to_string(&cargo)?;
                if let Ok(krate) = CrateNode::from_toml_str(&content, cargo.to_str().unwrap_or("")) {
                    crates.push(krate);
                }
            }
            // Don't recurse into target dirs.
            if path.file_name().map(|n| n != "target").unwrap_or(true) {
                scan_recursive(path.to_str().unwrap(), crates)?;
            }
        }
    }
    Ok(())
}
