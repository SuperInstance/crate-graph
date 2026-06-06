use crate_graph::*;
use anyhow::Result;
use clap::Parser;
use colored::*;

#[derive(Parser)]
#[command(name = "crate-graph", about = "Analyze Rust crate dependency graphs")]
struct Cli {
    /// Directory to scan for Cargo.toml files
    #[arg(default_value = ".")]
    dir: String,

    /// Show impact analysis for a specific crate
    #[arg(long)]
    impact: Option<String>,

    /// Show critical path
    #[arg(long)]
    critical_path: bool,

    /// Find bisimilar pairs
    #[arg(long)]
    bisimilar: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    println!("{}", "═".repeat(60).bright_blue());
    println!("{}", "  CRATE GRAPH ANALYZER".bright_blue().bold());
    println!("{}", "═".repeat(60).bright_blue());
    println!();

    println!("{} Scanning {} for Cargo.toml files...", "🔍".to_string(), cli.dir);
    let crates = scan_cargo_tomls(&cli.dir)?;
    println!("   Found {} crate manifests", crates.len().to_string().green());

    // Deduplicate by name (keep first seen).
    let mut seen = std::collections::HashSet::new();
    let crates: Vec<_> = crates
        .into_iter()
        .filter(|c| seen.insert(c.name.clone()))
        .collect();
    println!("   Unique crates: {}", crates.len().to_string().green());

    println!("\n{} Building dependency graph...", "📊".to_string());
    let (graph, cycles) = DependencyGraph::build_allow_cycles(crates)?;
    if !cycles.is_empty() {
        println!("   {} Cycles detected: {:?}", "⚠️".to_string(), cycles);
    }
    println!("   Nodes: {}", graph.node_count().to_string().green());
    println!("   Edges (in-fleet): {}", graph.edge_count().to_string().green());

    // Layer calculation.
    println!("\n{} Computing dependency layers...", "🧱".to_string());
    let layers = LayerCalculator::compute(&graph);
    println!("   Total layers: {}", layers.len().to_string().green());
    for (i, layer) in layers.iter().enumerate() {
        if i < 5 || i == layers.len() - 1 {
            println!(
                "   Layer {}: {} crates {}",
                i.to_string().yellow(),
                layer.len().to_string().green(),
                if layer.len() <= 8 {
                    format!("{:?}", layer.iter().take(8).collect::<Vec<_>>())
                } else {
                    format!("{:?} ...", layer.iter().take(5).collect::<Vec<_>>())
                }
            );
        } else if i == 5 {
            println!("   {}", "... (showing first 5 and last layer)".dimmed());
        }
    }

    // Critical path.
    println!("\n{} Analyzing critical path...", "⛓️".to_string());
    let (len, path) = CriticalPath::find(&graph);
    println!("   Longest chain: {}", len.to_string().green());
    println!("   Length: {}", len.to_string().green());
    if !path.is_empty() {
        println!(
            "   Path: {}",
            path.iter()
                .map(|s| s.cyan().to_string())
                .collect::<Vec<_>>()
                .join(" → ")
        );
    }

    // Impact analysis.
    println!("\n{} Computing impact analysis...", "💥".to_string());
    let analyzer = ImpactAnalyzer::new(&graph);
    let (most_dep, count) = analyzer.most_depended_on();
    println!("   Most-depended-on: {} ({} downstream)", most_dep.cyan(), count.to_string().red());

    // Top 10.
    println!("\n   {} Top 10 by downstream impact:", "🏆".to_string());
    let ranks = analyzer.rank_by_impact();
    for (i, (name, cnt)) in ranks.iter().take(10).enumerate() {
        println!(
            "   {:>2}. {} → {} downstream",
            (i + 1).to_string().yellow(),
            name.cyan(),
            cnt.to_string().red()
        );
    }

    // Specific crate impact.
    if let Some(crate_name) = &cli.impact {
        if graph.contains(crate_name) {
            let downstream = analyzer.downstream_count(crate_name);
            let layers = analyzer.downstream_by_depth(crate_name);
            println!("\n{} Impact analysis for {}", "🎯".to_string(), crate_name.cyan());
            println!("   Downstream crates: {}", downstream.to_string().red());
            for (i, layer) in layers.iter().enumerate() {
                println!(
                    "   Depth {}: {} crates {:?}",
                    (i + 1).to_string().yellow(),
                    layer.len().to_string().green(),
                    layer.iter().take(5).collect::<Vec<_>>()
                );
            }
        } else {
            println!("\n   {} Crate '{}' not found in fleet", "⚠️".to_string(), crate_name);
        }
    }

    // Critical path flag.
    if cli.critical_path {
        println!("\n{} Detailed critical path:", "⛓️".to_string());
        for (i, name) in path.iter().enumerate() {
            let layer = LayerCalculator::layer_of(&graph, name);
            println!(
                "   {}: {} (layer {})",
                i.to_string().yellow(),
                name.cyan(),
                layer.map(|l| l.to_string()).unwrap_or("?".to_string())
            );
        }
    }

    // Bisimulation.
    if cli.bisimilar {
        println!("\n{} Checking for bisimilar dependency subgraphs...", "🔬".to_string());
        let pairs = BisimulationChecker::find_bisimilar_pairs(&graph);
        if pairs.is_empty() {
            println!("   No bisimilar pairs found");
        } else {
            println!("   Found {} bisimilar pairs:", pairs.len().to_string().green());
            for (a, b) in pairs.iter().take(20) {
                println!("   {} ↔ {}", a.cyan(), b.cyan());
            }
        }
    }

    // Summary.
    println!("\n{}", "═".repeat(60).bright_blue());
    println!("{}", "  SUMMARY".bright_blue().bold());
    println!("{}", "═".repeat(60).bright_blue());
    println!("   Total crates:        {}", graph.node_count().to_string().green());
    println!("   Internal edges:      {}", graph.edge_count().to_string().green());
    println!("   Max layer:           {}", (layers.len() - 1).to_string().green());
    println!("   Longest path:        {} edges", len.to_string().green());
    println!("   Most-depended-on:    {} ({})", most_dep.cyan(), count.to_string().red());
    println!("   Layer 0 (leaf):      {} crates", layers.first().map(|l| l.len()).unwrap_or(0).to_string().green());
    println!();

    Ok(())
}
