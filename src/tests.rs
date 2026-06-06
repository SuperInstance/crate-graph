use crate::*;
use std::collections::HashSet;

fn make_crate(name: &str, deps: Vec<&str>) -> CrateNode {
    CrateNode {
        name: name.to_string(),
        version: "0.1.0".to_string(),
        dependencies: deps.into_iter().map(|s| s.to_string()).collect(),
        features: Vec::new(),
        path: format!("/fake/{}.toml", name),
    }
}

fn simple_graph() -> DependencyGraph {
    // A -> B -> C (C depends on B, B depends on A)
    let crates = vec![
        make_crate("a", vec![]),
        make_crate("b", vec!["a"]),
        make_crate("c", vec!["b"]),
    ];
    DependencyGraph::build(crates).unwrap()
}

fn diamond_graph() -> DependencyGraph {
    // A -> B, A -> C, B -> D, C -> D
    let crates = vec![
        make_crate("a", vec![]),
        make_crate("b", vec!["a"]),
        make_crate("c", vec!["a"]),
        make_crate("d", vec!["b", "c"]),
    ];
    DependencyGraph::build(crates).unwrap()
}

fn wide_graph() -> DependencyGraph {
    // Many crates depend on "core"
    let mut crates = vec![make_crate("core", vec![])];
    for i in 0..10 {
        crates.push(make_crate(&format!("lib{}", i), vec!["core"]));
    }
    DependencyGraph::build(crates).unwrap()
}

// ---- Tests ----

#[test]
fn test_parse_simple_cargo_toml() {
    let toml = r#"
[package]
name = "my-crate"
version = "1.2.3"

[dependencies]
serde = "1"
tokio = { version = "1", features = ["full"] }

[features]
default = ["json"]
json = []
"#;
    let krate = CrateNode::from_toml_str(toml, "/test/Cargo.toml").unwrap();
    assert_eq!(krate.name, "my-crate");
    assert_eq!(krate.version, "1.2.3");
    assert!(krate.dependencies.contains(&"serde".to_string()));
    assert!(krate.dependencies.contains(&"tokio".to_string()));
    assert!(krate.features.contains(&"json".to_string()));
}

#[test]
fn test_build_simple_graph() {
    let graph = simple_graph();
    assert_eq!(graph.node_count(), 3);
    assert_eq!(graph.edge_count(), 2);
}

#[test]
fn test_cycle_detection() {
    let crates = vec![
        make_crate("a", vec!["b"]),
        make_crate("b", vec!["c"]),
        make_crate("c", vec!["a"]),
    ];
    let result = DependencyGraph::build(crates);
    assert!(result.is_err());
    assert!(result.err().unwrap().to_string().contains("cycle"));
}

#[test]
fn test_allow_cycles() {
    let crates = vec![
        make_crate("a", vec!["b"]),
        make_crate("b", vec!["c"]),
        make_crate("c", vec!["a"]),
    ];
    let (graph, cycles) = DependencyGraph::build_allow_cycles(crates).unwrap();
    assert_eq!(graph.node_count(), 3);
    assert!(!cycles.is_empty());
}

#[test]
fn test_direct_dependents() {
    let graph = simple_graph();
    let deps = graph.direct_dependents("a");
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].name, "b");
}

#[test]
fn test_direct_dependencies() {
    let graph = simple_graph();
    let deps = graph.direct_dependencies("c");
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].name, "b");
}

#[test]
fn test_layers_simple() {
    let graph = simple_graph();
    let layers = LayerCalculator::compute(&graph);
    assert_eq!(layers.len(), 3);
    assert!(layers[0].contains("a"));
    assert!(layers[1].contains("b"));
    assert!(layers[2].contains("c"));
}

#[test]
fn test_layers_diamond() {
    let graph = diamond_graph();
    let layers = LayerCalculator::compute(&graph);
    assert_eq!(layers.len(), 3);
    assert!(layers[0].contains("a"));
    assert!(layers[1].contains("b"));
    assert!(layers[1].contains("c"));
    assert!(layers[2].contains("d"));
}

#[test]
fn test_layer_of() {
    let graph = diamond_graph();
    assert_eq!(LayerCalculator::layer_of(&graph, "a"), Some(0));
    assert_eq!(LayerCalculator::layer_of(&graph, "b"), Some(1));
    assert_eq!(LayerCalculator::layer_of(&graph, "d"), Some(2));
    assert_eq!(LayerCalculator::layer_of(&graph, "nonexistent"), None);
}

#[test]
fn test_critical_path_simple() {
    let graph = simple_graph();
    let (len, path) = CriticalPath::find(&graph);
    assert_eq!(len, 2); // 2 edges: a->b->c
    assert_eq!(path, vec!["a", "b", "c"]);
}

#[test]
fn test_critical_path_diamond() {
    let graph = diamond_graph();
    let (len, path) = CriticalPath::find(&graph);
    assert_eq!(len, 2); // a->b->d or a->c->d
    assert!(path.contains(&"a".to_string()));
    assert!(path.contains(&"d".to_string()));
}

#[test]
fn test_impact_analyzer() {
    let graph = simple_graph();
    let analyzer = ImpactAnalyzer::new(&graph);
    assert_eq!(analyzer.downstream_count("a"), 2); // b, c
    assert_eq!(analyzer.downstream_count("b"), 1); // c
    assert_eq!(analyzer.downstream_count("c"), 0);
}

#[test]
fn test_impact_by_depth() {
    let graph = diamond_graph();
    let analyzer = ImpactAnalyzer::new(&graph);
    let layers = analyzer.downstream_by_depth("a");
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].len(), 2); // b, c
    assert_eq!(layers[1].len(), 1); // d
}

#[test]
fn test_most_depended_on() {
    let graph = wide_graph();
    let analyzer = ImpactAnalyzer::new(&graph);
    let (name, count) = analyzer.most_depended_on();
    assert_eq!(name, "core");
    assert_eq!(count, 10);
}

#[test]
fn test_bisimulation_identical() {
    // Two crates with identical dep structure
    let crates = vec![
        make_crate("a", vec![]),
        make_crate("b", vec![]),
        make_crate("c", vec!["a"]),
        make_crate("d", vec!["b"]),
    ];
    let graph = DependencyGraph::build(crates).unwrap();
    assert!(BisimulationChecker::check(&graph, "c", "d"));
}

#[test]
fn test_bisimulation_different() {
    let crates = vec![
        make_crate("a", vec![]),
        make_crate("b", vec![]),
        make_crate("c", vec!["a"]),
        make_crate("d", vec!["a", "b"]),
    ];
    let graph = DependencyGraph::build(crates).unwrap();
    assert!(!BisimulationChecker::check(&graph, "c", "d"));
}

#[test]
fn test_external_deps_ignored() {
    // Dependencies not in the fleet should be silently ignored.
    let crates = vec![
        make_crate("a", vec!["serde", "tokio"]),
        make_crate("b", vec!["a"]),
    ];
    let graph = DependencyGraph::build(crates).unwrap();
    assert_eq!(graph.edge_count(), 1); // Only a->b
}

#[test]
fn test_dedupe_by_name() {
    let crates = vec![
        make_crate("a", vec![]),
        make_crate("a", vec![]),
        make_crate("b", vec!["a"]),
    ];
    let mut seen = HashSet::new();
    let unique: Vec<_> = crates.into_iter().filter(|c| seen.insert(c.name.clone())).collect();
    let graph = DependencyGraph::build(unique).unwrap();
    assert_eq!(graph.node_count(), 2);
}

#[test]
fn test_critical_path_through() {
    let graph = diamond_graph();
    let (_len, path) = CriticalPath::find_through(&graph, "b");
    assert!(path.contains(&"b".to_string()));
    assert!(path.contains(&"d".to_string()));
}

#[test]
fn test_rank_by_impact() {
    let graph = diamond_graph();
    let analyzer = ImpactAnalyzer::new(&graph);
    let ranks = analyzer.rank_by_impact();
    assert_eq!(ranks[0].0, "a"); // Most downstream impact
    assert!(ranks[0].1 >= 3); // b, c, d
}

#[test]
fn test_find_bisimilar_pairs() {
    let crates = vec![
        make_crate("base1", vec![]),
        make_crate("base2", vec![]),
        make_crate("leaf1", vec!["base1"]),
        make_crate("leaf2", vec!["base2"]),
    ];
    let graph = DependencyGraph::build(crates).unwrap();
    let pairs = BisimulationChecker::find_bisimilar_pairs(&graph);
    // leaf1 and leaf2 should be bisimilar, base1 and base2 should be bisimilar
    assert!(!pairs.is_empty());
}
