# crate-graph

Dependency graph analysis for Rust crate fleets. Parses Cargo.toml files across a directory and builds a directed dependency graph with cycle detection, layer computation, critical path analysis, impact analysis, and bisimulation checking.

## Components

- **`CrateNode`** — Rust crate metadata parsed from Cargo.toml
- **`DependencyGraph`** — DAG builder with cycle detection from a fleet of Cargo.toml files
- **`LayerCalculator`** — Computes dependency layers (layer 0 = no deps, layer N = depends on layer N-1)
- **`CriticalPath`** — Finds the longest dependency chain in the fleet
- **`ImpactAnalyzer`** — Blast radius analysis: how many downstream crates affected by a change
- **`BisimulationChecker`** — Checks if two dependency subgraphs are structurally identical (borrowed from category theory)

## Usage

```bash
# Analyze current directory
cargo run

# Analyze a specific directory
cargo run -- /path/to/repos

# Show critical path details
cargo run -- /path/to/repos --critical-path

# Find structurally identical subgraphs
cargo run -- /path/to/repos --bisimilar

# Impact analysis for a specific crate
cargo run -- /path/to/repos --impact crate-name
```

## Real Stats (run against 553 Cargo.toml files)

- **Total crates:** 1,061
- **Internal edges:** 3,701
- **Max layer:** 5
- **Longest path:** 36 edges (through Zed editor crate tree)
- **Most-depended-on:** tokio-stream (261 downstream crates)
- **Bisimilar pairs:** 172,601 structurally identical dependency subgraphs

## Tests

21 tests covering all components: parsing, graph building, cycle detection, layers, critical path, impact analysis, bisimulation, edge cases.

```bash
cargo test
```
