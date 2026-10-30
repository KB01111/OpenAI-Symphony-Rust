//! The indexed graph in memory: the views the UI draws and the traversals that answer "what
//! depends on this".
//!
//! A repository's graph is thousands of nodes and tens of thousands of edges, which is small
//! enough to hold in memory and walk directly; SQLite only persists it.

use crate::engine::is_dependency_relation;
use crate::model::{
    AffectedNode, Confidence, GraphEdge, GraphLevel, GraphNode, GraphView, ModuleStat, NodeDetail,
    NodeKind, NodeNeighbor,
};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

/// Nodes in any one response; larger graphs are cut to the best-connected nodes.
pub const MAX_VIEW_NODES: usize = 1500;
pub const MAX_VIEW_EDGES: usize = 6000;
const DETAIL_LIMIT: usize = 60;
const MODULE_EXTERNALS: usize = 25;

pub struct LoadedGraph {
    pub run_id: i64,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    index: HashMap<String, usize>,
    outgoing: Vec<Vec<usize>>,
    incoming: Vec<Vec<usize>>,
    /// Symbol nodes of each file, by line.
    symbols_by_file: HashMap<String, Vec<usize>>,
    file_nodes: HashMap<String, usize>,
}

fn dependency_edge(edge: &GraphEdge) -> bool {
    is_dependency_relation(&edge.relation)
}

impl LoadedGraph {
    pub fn new(run_id: i64, nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> Self {
        let index: HashMap<String, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        let mut outgoing = vec![Vec::new(); nodes.len()];
        let mut incoming = vec![Vec::new(); nodes.len()];
        for (i, edge) in edges.iter().enumerate() {
            if let (Some(&s), Some(&t)) = (index.get(&edge.source), index.get(&edge.target)) {
                outgoing[s].push(i);
                incoming[t].push(i);
            }
        }
        let mut symbols_by_file: HashMap<String, Vec<usize>> = HashMap::new();
        let mut file_nodes = HashMap::new();
        for (i, node) in nodes.iter().enumerate() {
            let Some(file) = node.file.as_ref() else {
                continue;
            };
            if node.kind == NodeKind::File {
                file_nodes.insert(file.clone(), i);
            } else if node.kind.is_symbol() {
                symbols_by_file.entry(file.clone()).or_default().push(i);
            }
        }
        for list in symbols_by_file.values_mut() {
            list.sort_by_key(|&i| (nodes[i].line.unwrap_or(0), nodes[i].id.clone()));
        }
        Self {
            run_id,
            nodes,
            edges,
            index,
            outgoing,
            incoming,
            symbols_by_file,
            file_nodes,
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn nodes(&self) -> &[GraphNode] {
        &self.nodes
    }

    pub fn node(&self, id: &str) -> Option<&GraphNode> {
        self.index.get(id).map(|&i| &self.nodes[i])
    }

    pub fn file_node(&self, file: &str) -> Option<&GraphNode> {
        self.file_nodes.get(file).map(|&i| &self.nodes[i])
    }

    /// The symbols of a file, ordered by line.
    pub fn symbols_in_file(&self, file: &str) -> Vec<&GraphNode> {
        self.symbols_by_file
            .get(file)
            .map(|v| v.iter().map(|&i| &self.nodes[i]).collect())
            .unwrap_or_default()
    }

    /// Find a symbol by what identifies it when the node id changed (a renamed file).
    pub fn find_symbol(&self, file: &str, label: &str, kind: NodeKind) -> Option<&GraphNode> {
        self.symbols_in_file(file)
            .into_iter()
            .find(|n| n.label == label && n.kind == kind)
    }

    fn view(
        &self,
        level: GraphLevel,
        focus: Option<String>,
        nodes: Vec<GraphNode>,
        edges: Vec<GraphEdge>,
        total_nodes: usize,
        total_edges: usize,
    ) -> GraphView {
        let truncated = nodes.len() < total_nodes || edges.len() < total_edges;
        GraphView {
            run_id: self.run_id,
            level,
            focus,
            nodes,
            edges,
            total_nodes: total_nodes as u32,
            total_edges: total_edges as u32,
            truncated,
        }
    }

    // -- module level -----------------------------------------------------------------------

    /// One node per package / directory module with aggregated dependency edges between them.
    pub fn module_view(&self, include_external: bool) -> GraphView {
        struct Module {
            label: String,
            package: bool,
            files: u32,
            symbols: u32,
            degree: u32,
        }
        let mut modules: BTreeMap<String, Module> = BTreeMap::new();
        for node in &self.nodes {
            let Some(module) = node.module.as_ref() else {
                continue;
            };
            let entry = modules.entry(module.clone()).or_insert_with(|| Module {
                label: module.rsplit('/').next().unwrap_or(module).to_string(),
                package: false,
                files: 0,
                symbols: 0,
                degree: 0,
            });
            match node.kind {
                NodeKind::File => entry.files += 1,
                NodeKind::Package => {
                    entry.package = true;
                    entry.label = node.label.clone();
                }
                k if k.is_symbol() => entry.symbols += 1,
                _ => {}
            }
        }

        let mut weights: BTreeMap<(String, String), (u32, Confidence)> = BTreeMap::new();
        let mut external: BTreeMap<(String, String), (u32, Confidence)> = BTreeMap::new();
        for edge in self.edges.iter().filter(|e| dependency_edge(e)) {
            let (Some(&s), Some(&t)) = (self.index.get(&edge.source), self.index.get(&edge.target))
            else {
                continue;
            };
            let (a, b) = (&self.nodes[s], &self.nodes[t]);
            let Some(from) = a.module.as_ref() else {
                continue;
            };
            match (&b.module, b.kind) {
                (Some(to), _) if to != from => {
                    let slot = weights
                        .entry((from.clone(), to.clone()))
                        .or_insert((0, Confidence::Ambiguous));
                    slot.0 += 1;
                    if edge.confidence.rank() > slot.1.rank() {
                        slot.1 = edge.confidence;
                    }
                }
                (None, NodeKind::Dependency) if include_external => {
                    let slot = external
                        .entry((from.clone(), b.id.clone()))
                        .or_insert((0, Confidence::Ambiguous));
                    slot.0 += 1;
                    if edge.confidence.rank() > slot.1.rank() {
                        slot.1 = edge.confidence;
                    }
                }
                _ => {}
            }
        }
        for ((from, to), (weight, _)) in &weights {
            if let Some(m) = modules.get_mut(from) {
                m.degree += weight;
            }
            if let Some(m) = modules.get_mut(to) {
                m.degree += weight;
            }
        }

        let mut nodes: Vec<GraphNode> = modules
            .iter()
            .map(|(id, m)| GraphNode {
                id: id.clone(),
                label: m.label.clone(),
                kind: if m.package {
                    NodeKind::Package
                } else {
                    NodeKind::Module
                },
                file: None,
                line: None,
                module: Some(id.clone()),
                degree: m.degree,
                members: m.files.max(1),
            })
            .collect();
        let mut edges: Vec<GraphEdge> = weights
            .into_iter()
            .map(|((source, target), (weight, confidence))| GraphEdge {
                source,
                target,
                relation: "depends".into(),
                confidence,
                line: None,
                weight,
            })
            .collect();

        if include_external {
            let mut by_dep: BTreeMap<String, u32> = BTreeMap::new();
            for ((_, dep), (weight, _)) in &external {
                *by_dep.entry(dep.clone()).or_default() += weight;
            }
            let mut ranked: Vec<(String, u32)> = by_dep.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            ranked.truncate(MODULE_EXTERNALS);
            let kept: HashSet<&str> = ranked.iter().map(|(id, _)| id.as_str()).collect();
            for (id, weight) in &ranked {
                if let Some(node) = self.node(id) {
                    let mut node = node.clone();
                    node.degree = *weight;
                    nodes.push(node);
                }
            }
            for ((from, dep), (weight, confidence)) in external {
                if kept.contains(dep.as_str()) {
                    edges.push(GraphEdge {
                        source: from,
                        target: dep,
                        relation: "depends".into(),
                        confidence,
                        line: None,
                        weight,
                    });
                }
            }
        }

        let (total_nodes, total_edges) = (nodes.len(), edges.len());
        if nodes.len() > MAX_VIEW_NODES {
            nodes.sort_by(|a, b| b.degree.cmp(&a.degree).then_with(|| a.id.cmp(&b.id)));
            nodes.truncate(MAX_VIEW_NODES);
            let keep: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
            edges.retain(|e| keep.contains(e.source.as_str()) && keep.contains(e.target.as_str()));
        }
        self.view(
            GraphLevel::Modules,
            None,
            nodes,
            edges,
            total_nodes,
            total_edges,
        )
    }

    // -- file level -------------------------------------------------------------------------

    /// Files and the dependencies between them, lifted from the symbol-level edges. With
    /// `module`, the files of that module plus the files they directly depend on or are used by.
    pub fn file_view(&self, module: Option<&str>) -> GraphView {
        let file_of = |i: usize| self.nodes[i].file.as_deref();
        let in_module = |file: &str| -> bool {
            match module {
                None => true,
                Some(m) => self
                    .file_nodes
                    .get(file)
                    .map(|&i| self.nodes[i].module.as_deref() == Some(m))
                    .unwrap_or(false),
            }
        };
        let mut weights: BTreeMap<(String, String), (u32, Confidence)> = BTreeMap::new();
        for edge in self.edges.iter().filter(|e| dependency_edge(e)) {
            let (Some(&s), Some(&t)) = (self.index.get(&edge.source), self.index.get(&edge.target))
            else {
                continue;
            };
            let (Some(from), Some(to)) = (file_of(s), file_of(t)) else {
                continue;
            };
            if from == to
                || !self.file_nodes.contains_key(from)
                || !self.file_nodes.contains_key(to)
            {
                continue;
            }
            if module.is_some() && !(in_module(from) || in_module(to)) {
                continue;
            }
            let slot = weights
                .entry((from.to_string(), to.to_string()))
                .or_insert((0, Confidence::Ambiguous));
            slot.0 += 1;
            if edge.confidence.rank() > slot.1.rank() {
                slot.1 = edge.confidence;
            }
        }
        let mut wanted: HashSet<&str> = HashSet::new();
        for (from, to) in weights.keys() {
            wanted.insert(from);
            wanted.insert(to);
        }
        if let Some(m) = module {
            // Files of the module that nothing links to are still part of it.
            for (file, &i) in &self.file_nodes {
                if self.nodes[i].module.as_deref() == Some(m) {
                    wanted.insert(file);
                }
            }
        } else {
            for file in self.file_nodes.keys() {
                wanted.insert(file);
            }
        }
        let mut nodes: Vec<GraphNode> = wanted
            .iter()
            .filter_map(|f| self.file_nodes.get(*f).map(|&i| self.nodes[i].clone()))
            .collect();
        let mut degree: HashMap<&str, u32> = HashMap::new();
        for ((from, to), (weight, _)) in &weights {
            *degree.entry(from.as_str()).or_default() += weight;
            *degree.entry(to.as_str()).or_default() += weight;
        }
        for node in &mut nodes {
            node.degree = node
                .file
                .as_deref()
                .and_then(|f| degree.get(f).copied())
                .unwrap_or(0);
        }
        let mut edges: Vec<GraphEdge> = weights
            .into_iter()
            .map(|((source, target), (weight, confidence))| GraphEdge {
                source: self
                    .file_nodes
                    .get(&source)
                    .map(|&i| self.nodes[i].id.clone())
                    .unwrap_or(source),
                target: self
                    .file_nodes
                    .get(&target)
                    .map(|&i| self.nodes[i].id.clone())
                    .unwrap_or(target),
                relation: "depends".into(),
                confidence,
                line: None,
                weight,
            })
            .collect();
        let (total_nodes, total_edges) = (nodes.len(), edges.len());
        nodes.sort_by(|a, b| b.degree.cmp(&a.degree).then_with(|| a.id.cmp(&b.id)));
        if nodes.len() > MAX_VIEW_NODES {
            nodes.truncate(MAX_VIEW_NODES);
        }
        let keep: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        edges.retain(|e| keep.contains(e.source.as_str()) && keep.contains(e.target.as_str()));
        edges.sort_by_key(|e| std::cmp::Reverse(e.weight));
        edges.truncate(MAX_VIEW_EDGES);
        self.view(
            GraphLevel::Files,
            module.map(str::to_string),
            nodes,
            edges,
            total_nodes,
            total_edges,
        )
    }

    // -- symbol level -----------------------------------------------------------------------

    /// The neighbourhood of a node (a file's symbols, or a symbol with what it uses and what uses
    /// it) up to `depth` hops, strongest connections first when it has to be cut.
    pub fn neighborhood(
        &self,
        focus: &str,
        depth: u32,
        include_inferred: bool,
    ) -> Option<GraphView> {
        let start = *self.index.get(focus)?;
        let mut seeds = vec![start];
        if self.nodes[start].kind == NodeKind::File {
            if let Some(file) = self.nodes[start].file.as_deref() {
                seeds.extend(self.symbols_by_file.get(file).cloned().unwrap_or_default());
            }
        }
        let mut seen: HashSet<usize> = seeds.iter().copied().collect();
        let mut frontier: VecDeque<(usize, u32)> = seeds.iter().map(|&i| (i, 0)).collect();
        let mut kept_edges: Vec<usize> = Vec::new();
        let mut total_edges = 0usize;
        while let Some((node, hops)) = frontier.pop_front() {
            if hops >= depth.clamp(1, 3) {
                continue;
            }
            for &e in self.outgoing[node].iter().chain(self.incoming[node].iter()) {
                let edge = &self.edges[e];
                if !include_inferred && edge.confidence != Confidence::Extracted {
                    continue;
                }
                total_edges += 1;
                let other = if edge.source == self.nodes[node].id {
                    &edge.target
                } else {
                    &edge.source
                };
                let Some(&other_index) = self.index.get(other) else {
                    continue;
                };
                if seen.len() >= MAX_VIEW_NODES && !seen.contains(&other_index) {
                    continue;
                }
                kept_edges.push(e);
                if seen.insert(other_index) {
                    frontier.push_back((other_index, hops + 1));
                }
            }
        }
        let total_nodes = seen.len();
        let mut ordered: Vec<usize> = seen.into_iter().collect();
        ordered.sort_unstable();
        let nodes: Vec<GraphNode> = ordered.iter().map(|&i| self.nodes[i].clone()).collect();
        let present: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        let mut seen_edges = HashSet::new();
        let mut edges: Vec<GraphEdge> = Vec::new();
        for e in kept_edges {
            let edge = &self.edges[e];
            if present.contains(edge.source.as_str())
                && present.contains(edge.target.as_str())
                && seen_edges.insert(e)
            {
                edges.push(edge.clone());
            }
        }
        edges.truncate(MAX_VIEW_EDGES);
        Some(self.view(
            GraphLevel::Symbols,
            Some(focus.to_string()),
            nodes,
            edges,
            total_nodes,
            total_edges.max(seen_edges.len()),
        ))
    }

    // -- node detail ------------------------------------------------------------------------

    /// What a node uses, what uses it and what it contains. Commits are added by the caller (they
    /// come from git, not from the index).
    pub fn detail(&self, id: &str) -> Option<NodeDetail> {
        let &i = self.index.get(id)?;
        let neighbor = |edge: &GraphEdge, other: &str| -> Option<NodeNeighbor> {
            Some(NodeNeighbor {
                node: self.node(other)?.clone(),
                relation: edge.relation.clone(),
                confidence: edge.confidence,
                line: edge.line,
            })
        };
        let rank = |n: &NodeNeighbor| {
            (
                std::cmp::Reverse(n.confidence.rank()),
                std::cmp::Reverse(n.node.degree),
            )
        };
        let mut dependencies: Vec<NodeNeighbor> = self.outgoing[i]
            .iter()
            .map(|&e| &self.edges[e])
            .filter(|e| dependency_edge(e))
            .filter_map(|e| neighbor(e, &e.target))
            .collect();
        let mut references: Vec<NodeNeighbor> = self.incoming[i]
            .iter()
            .map(|&e| &self.edges[e])
            .filter(|e| dependency_edge(e))
            .filter_map(|e| neighbor(e, &e.source))
            .collect();
        dependencies.sort_by_key(rank);
        references.sort_by_key(rank);
        let (dependencies_total, references_total) =
            (dependencies.len() as u32, references.len() as u32);
        dependencies.truncate(DETAIL_LIMIT);
        references.truncate(DETAIL_LIMIT);

        let mut members: Vec<GraphNode> = if self.nodes[i].kind == NodeKind::File {
            self.nodes[i]
                .file
                .as_deref()
                .map(|f| self.symbols_in_file(f).into_iter().cloned().collect())
                .unwrap_or_default()
        } else {
            self.outgoing[i]
                .iter()
                .map(|&e| &self.edges[e])
                .filter(|e| {
                    matches!(
                        e.relation.as_str(),
                        "contains" | "method" | "case_of" | "declares"
                    )
                })
                .filter_map(|e| self.node(&e.target).cloned())
                .collect()
        };
        members.sort_by_key(|n| n.line.unwrap_or(0));
        members.truncate(DETAIL_LIMIT * 2);

        Some(NodeDetail {
            node: self.nodes[i].clone(),
            dependencies,
            references,
            members,
            dependencies_total,
            references_total,
            recent_commits: Vec::new(),
        })
    }

    // -- overview ---------------------------------------------------------------------------

    /// Per-module counts and aggregated dependency weights, busiest first.
    pub fn module_stats(&self, limit: usize) -> Vec<ModuleStat> {
        let mut stats: BTreeMap<String, ModuleStat> = BTreeMap::new();
        for node in &self.nodes {
            let Some(module) = node.module.as_ref() else {
                continue;
            };
            let entry = stats.entry(module.clone()).or_insert_with(|| ModuleStat {
                module: module.clone(),
                files: 0,
                symbols: 0,
                incoming: 0,
                outgoing: 0,
            });
            if node.kind == NodeKind::File {
                entry.files += 1;
            } else if node.kind.is_symbol() {
                entry.symbols += 1;
            }
        }
        for edge in self.module_view(false).edges {
            if let Some(m) = stats.get_mut(&edge.source) {
                m.outgoing += edge.weight;
            }
            if let Some(m) = stats.get_mut(&edge.target) {
                m.incoming += edge.weight;
            }
        }
        let mut list: Vec<ModuleStat> = stats.into_values().collect();
        list.sort_by(|a, b| {
            (b.files + b.symbols)
                .cmp(&(a.files + a.symbols))
                .then_with(|| a.module.cmp(&b.module))
        });
        list.truncate(limit);
        list
    }

    /// Types, functions and methods in the index.
    pub fn symbol_count(&self) -> u32 {
        self.nodes.iter().filter(|n| n.kind.is_symbol()).count() as u32
    }

    /// The most depended-upon files and symbols.
    pub fn hubs(&self, limit: usize) -> Vec<GraphNode> {
        let mut list: Vec<&GraphNode> = self
            .nodes
            .iter()
            .filter(|n| n.kind.is_symbol() || n.kind == NodeKind::File)
            .collect();
        list.sort_by(|a, b| b.degree.cmp(&a.degree).then_with(|| a.id.cmp(&b.id)));
        list.into_iter().take(limit).cloned().collect()
    }

    /// Share of dependency edges that were inferred rather than read from source, with the totals.
    pub fn inferred_share(&self) -> (u32, u32) {
        let deps: Vec<&GraphEdge> = self.edges.iter().filter(|e| dependency_edge(e)).collect();
        let inferred = deps
            .iter()
            .filter(|e| e.confidence != Confidence::Extracted)
            .count();
        (inferred as u32, deps.len() as u32)
    }

    // -- impact -----------------------------------------------------------------------------

    /// Everything that depends on `changed`, directly or through up to `max_depth` hops, found by
    /// walking dependency edges backwards. A node's confidence is that of the weakest edge on the
    /// path that reached it. Returns the affected nodes (not the changed ones) and whether
    /// `max_nodes` cut the result.
    pub fn affected(
        &self,
        changed: &[String],
        max_depth: u32,
        max_nodes: usize,
        include_inferred: bool,
    ) -> (Vec<AffectedNode>, bool) {
        let mut visited: HashMap<usize, (u32, Confidence)> = HashMap::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        for id in changed {
            if let Some(&i) = self.index.get(id) {
                visited.insert(i, (0, Confidence::Extracted));
                queue.push_back(i);
            }
        }
        let changed_set: HashSet<usize> = visited.keys().copied().collect();
        let mut found: Vec<AffectedNode> = Vec::new();
        let mut truncated = false;
        'walk: while let Some(node) = queue.pop_front() {
            let (depth, confidence) = visited[&node];
            if depth >= max_depth {
                continue;
            }
            for &e in &self.incoming[node] {
                let edge = &self.edges[e];
                if !dependency_edge(edge)
                    || (!include_inferred && edge.confidence != Confidence::Extracted)
                {
                    continue;
                }
                let Some(&from) = self.index.get(&edge.source) else {
                    continue;
                };
                if visited.contains_key(&from) || changed_set.contains(&from) {
                    continue;
                }
                // An external package does not depend on the repository.
                if self.nodes[from].kind == NodeKind::Dependency {
                    continue;
                }
                if found.len() >= max_nodes {
                    truncated = true;
                    break 'walk;
                }
                let reached = confidence.weakest(edge.confidence);
                visited.insert(from, (depth + 1, reached));
                queue.push_back(from);
                found.push(AffectedNode {
                    node: self.nodes[from].clone(),
                    depth: depth + 1,
                    via_relation: edge.relation.clone(),
                    from_node_id: self.nodes[node].id.clone(),
                    confidence: reached,
                });
            }
        }
        found.sort_by(|a, b| {
            a.depth
                .cmp(&b.depth)
                .then_with(|| b.confidence.rank().cmp(&a.confidence.rank()))
                .then_with(|| b.node.degree.cmp(&a.node.degree))
                .then_with(|| a.node.id.cmp(&b.node.id))
        });
        (found, truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: &str, kind: NodeKind, file: &str, module: &str, line: u32) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: id.into(),
            kind,
            file: Some(file.into()),
            line: Some(line),
            module: Some(module.into()),
            degree: 0,
            members: 1,
        }
    }

    fn e(source: &str, target: &str, relation: &str, confidence: Confidence) -> GraphEdge {
        GraphEdge {
            source: source.into(),
            target: target.into(),
            relation: relation.into(),
            confidence,
            line: Some(1),
            weight: 1,
        }
    }

    /// core::file_a { fn_a }   server::file_b { fn_b -> fn_a (calls) , fn_c -> fn_b (inferred) }
    /// cli::file_c { fn_d -> fn_c }  plus an external package used by fn_b.
    fn sample() -> LoadedGraph {
        let mut ext = n("serde", NodeKind::Dependency, "", "", 0);
        ext.file = None;
        ext.module = None;
        let nodes = vec![
            n("file_a", NodeKind::File, "core/a.rs", "core", 1),
            n("fn_a", NodeKind::Function, "core/a.rs", "core", 10),
            n("file_b", NodeKind::File, "server/b.rs", "server", 1),
            n("fn_b", NodeKind::Function, "server/b.rs", "server", 5),
            n("fn_c", NodeKind::Function, "server/b.rs", "server", 20),
            n("file_c", NodeKind::File, "cli/c.rs", "cli", 1),
            n("fn_d", NodeKind::Function, "cli/c.rs", "cli", 3),
            ext,
        ];
        let edges = vec![
            e("file_a", "fn_a", "contains", Confidence::Extracted),
            e("file_b", "fn_b", "contains", Confidence::Extracted),
            e("fn_b", "fn_a", "calls", Confidence::Extracted),
            e("fn_c", "fn_b", "calls", Confidence::Inferred),
            e("fn_d", "fn_c", "calls", Confidence::Extracted),
            e("fn_b", "serde", "imports_from", Confidence::Extracted),
        ];
        LoadedGraph::new(7, nodes, edges)
    }

    #[test]
    fn affected_walks_dependents_and_keeps_the_weakest_confidence() {
        let graph = sample();
        let (affected, truncated) = graph.affected(&["fn_a".into()], 3, 100, true);
        assert!(!truncated);
        let ids: Vec<_> = affected
            .iter()
            .map(|a| (a.node.id.as_str(), a.depth, a.confidence))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("fn_b", 1, Confidence::Extracted),
                ("fn_c", 2, Confidence::Inferred),
                // fn_d reaches fn_a only through the inferred call, so it is inferred too.
                ("fn_d", 3, Confidence::Inferred),
            ]
        );
        assert_eq!(affected[2].from_node_id, "fn_c");
        assert_eq!(affected[0].via_relation, "calls");

        // Depth limits the walk; extracted-only drops the inferred branch entirely.
        assert_eq!(graph.affected(&["fn_a".into()], 1, 100, true).0.len(), 1);
        let (strong, _) = graph.affected(&["fn_a".into()], 3, 100, false);
        assert_eq!(
            strong
                .iter()
                .map(|a| a.node.id.as_str())
                .collect::<Vec<_>>(),
            vec!["fn_b"]
        );
        // The external package is never "affected", and structural edges do not propagate.
        let (from_ext, _) = graph.affected(&["serde".into()], 3, 100, true);
        assert_eq!(
            from_ext
                .iter()
                .map(|a| a.node.id.as_str())
                .collect::<Vec<_>>(),
            vec!["fn_b", "fn_c", "fn_d"]
        );
        assert!(graph
            .affected(&["file_a".into()], 3, 100, true)
            .0
            .is_empty());
    }

    #[test]
    fn the_affected_set_is_capped_and_says_so() {
        let graph = sample();
        let (affected, truncated) = graph.affected(&["fn_a".into()], 3, 2, true);
        assert_eq!(affected.len(), 2);
        assert!(truncated);
    }

    #[test]
    fn module_view_aggregates_edges_between_modules() {
        let graph = sample();
        let view = graph.module_view(false);
        assert_eq!(view.nodes.len(), 3);
        let edge = |a: &str, b: &str| view.edges.iter().find(|e| e.source == a && e.target == b);
        assert_eq!(edge("server", "core").unwrap().weight, 1);
        assert_eq!(
            edge("cli", "server").unwrap().confidence,
            Confidence::Extracted
        );
        assert!(view.edges.iter().all(|e| e.source != e.target));
        assert!(!view.nodes.iter().any(|n| n.id == "serde"));
        let with_external = graph.module_view(true);
        assert!(with_external.nodes.iter().any(|n| n.id == "serde"));
        assert!(with_external
            .edges
            .iter()
            .any(|e| e.source == "server" && e.target == "serde"));
        let server = view.nodes.iter().find(|n| n.id == "server").unwrap();
        assert_eq!(server.members, 1);
    }

    #[test]
    fn file_view_lifts_symbol_edges_to_files() {
        let graph = sample();
        let view = graph.file_view(None);
        assert_eq!(view.nodes.len(), 3);
        let edge = view
            .edges
            .iter()
            .find(|e| e.source == "file_b" && e.target == "file_a")
            .unwrap();
        assert_eq!(edge.weight, 1);
        let scoped = graph.file_view(Some("core"));
        // core's own file plus the file that uses it; cli is two hops away.
        let ids: HashSet<_> = scoped.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains("file_a") && ids.contains("file_b") && !ids.contains("file_c"));
    }

    #[test]
    fn neighborhood_of_a_file_includes_its_symbols_and_their_neighbours() {
        let graph = sample();
        let view = graph.neighborhood("file_b", 1, true).unwrap();
        let ids: HashSet<_> = view.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(
            ids.contains("fn_b")
                && ids.contains("fn_c")
                && ids.contains("fn_a")
                && ids.contains("serde")
        );
        // fn_d calls a symbol of the file, so it is a neighbour; its own file is two hops away.
        assert!(ids.contains("fn_d") && !ids.contains("file_c"));
        assert!(graph.neighborhood("missing", 1, true).is_none());
        // Without inferred edges the fn_c <-> fn_b relation is not followed.
        let strong = graph.neighborhood("fn_b", 1, false).unwrap();
        assert!(!strong.nodes.iter().any(|n| n.id == "fn_c"));
    }

    #[test]
    fn detail_lists_dependencies_references_and_members() {
        let graph = sample();
        let detail = graph.detail("fn_b").unwrap();
        assert_eq!(detail.dependencies_total, 2);
        assert_eq!(detail.references_total, 1);
        assert_eq!(detail.references[0].node.id, "fn_c");
        let file = graph.detail("file_b").unwrap();
        assert_eq!(
            file.members
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["fn_b", "fn_c"]
        );
        assert_eq!(graph.symbols_in_file("server/b.rs").len(), 2);
        assert!(graph
            .find_symbol("server/b.rs", "fn_c", NodeKind::Function)
            .is_some());
    }
}
