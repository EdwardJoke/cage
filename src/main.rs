use std::collections::HashMap;
use std::path::Path;

use clap::{Args, Parser, Subcommand};

use coplex_cage::orchestrator::{Orchestrator, OrchestratorConfig};
use coplex_cage::router::{RouterConfig, Topology};
use coplex_cage::sandbox;

#[derive(Parser)]
#[command(name = "cage", about = "Agent WASM sandbox", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Load and run a single WASM agent
    Run(RunArgs),
    /// Multi-agent orchestration
    Orchestrate(OrchestrateArgs),
    /// Resume from a checkpoint
    Resume(ResumeArgs),
}

#[derive(Args)]
struct RunArgs {
    /// Path to the WASM agent module
    agent: String,

    /// JSON payload to send to the agent on init
    #[arg(short, long)]
    message: Option<String>,

    /// Environment variables injected into the agent (KEY=VALUE)
    #[arg(short = 'e', long = "env", value_name = "KEY=VALUE")]
    env: Vec<String>,

    /// Allow the agent to make HTTP requests to URLs with this prefix
    #[arg(long = "allow-url", value_name = "PREFIX")]
    allow_url: Vec<String>,

    /// Maximum fuel (instructions) the agent can consume
    #[arg(short, long, default_value = "200000")]
    fuel: u64,

    /// Enable verbose (debug) logging
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Args)]
struct OrchestrateArgs {
    /// Agent specifications: id=path e.g. "leader=examples/agent-p0/target/.../agent_p0.wasm"
    #[arg(long = "agent", short = 'a', required = true)]
    agents: Vec<String>,

    /// JSON init payload (applied to all agents)
    #[arg(short, long)]
    message: Option<String>,

    /// Environment variables (applied to all agents): KEY=VALUE
    #[arg(short = 'e', long = "env")]
    env: Vec<String>,

    /// Allowed URL prefixes (applied to all agents)
    #[arg(long = "allow-url")]
    allow_url: Vec<String>,

    /// Fuel per agent
    #[arg(short, long, default_value = "500000")]
    fuel: u64,

    /// Number of tick rounds to execute
    #[arg(short, long, default_value = "1")]
    rounds: u32,

    /// Enable verbose (debug) logging
    #[arg(short, long)]
    verbose: bool,

    /// Routing topology: direct, broadcast, pattern, hub-and-spoke
    #[arg(long, default_value = "direct")]
    topology: String,

    /// Hub agent ID (required when --topology hub-and-spoke)
    #[arg(long)]
    hub: Option<String>,

    /// Enable Dead Letter Queue for unroutable messages
    #[arg(long)]
    dlq: bool,

    /// Save checkpoint after every N rounds
    #[arg(long)]
    save_every: Option<usize>,

    /// Directory for checkpoint files
    #[arg(long)]
    checkpoint_dir: Option<String>,

    /// Include WASM linear memory in checkpoints
    #[arg(long)]
    full_snapshot: bool,
}

#[derive(Args)]
struct ResumeArgs {
    /// Path to checkpoint JSON file
    #[arg(short, long)]
    checkpoint: String,

    /// Number of tick rounds to execute
    #[arg(short, long, default_value = "1")]
    rounds: u32,

    /// Enable verbose (debug) logging
    #[arg(short, long)]
    verbose: bool,

    /// Save checkpoint after every N rounds
    #[arg(long)]
    save_every: Option<usize>,

    /// Directory for checkpoint files
    #[arg(long)]
    checkpoint_dir: Option<String>,

    /// Include WASM linear memory in checkpoints
    #[arg(long)]
    full_snapshot: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Run(args) => run(args),
        Command::Orchestrate(args) => orchestrate(args),
        Command::Resume(args) => resume(args),
    }
}

fn run(args: RunArgs) -> anyhow::Result<()> {
    let log_level = if args.verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    env_logger::Builder::new()
        .filter_level(log_level)
        .init();

    let mut cage = sandbox::Sandbox::new()?;

    for pair in &args.env {
        if let Some((k, v)) = pair.split_once('=') {
            cage.set_env(k, v);
        } else {
            eprintln!("warning: ignoring malformed --env '{pair}' (expected KEY=VALUE)");
        }
    }

    for url in &args.allow_url {
        cage.allow_url(url);
    }

    cage.load_agent(&args.agent, args.fuel)?;

    if let Some(msg) = cage.init(args.message.as_deref())? {
        println!("init response: {msg:?}");
    }

    if let Some(msg) = cage.tick()? {
        println!("tick response: {msg:?}");
    }

    Ok(())
}

fn orchestrate(args: OrchestrateArgs) -> anyhow::Result<()> {
    let log_level = if args.verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    env_logger::Builder::new()
        .filter_level(log_level)
        .init();

    // Build router config from CLI flags
    let topology = match args.topology.as_str() {
        "direct" => Topology::Direct,
        "broadcast" => Topology::Broadcast,
        "pattern" => Topology::Pattern,
        "hub-and-spoke" => Topology::HubAndSpoke,
        other => {
            eprintln!("error: unknown topology '{other}'. Valid: direct, broadcast, pattern, hub-and-spoke");
            std::process::exit(1);
        }
    };
    let router_config = RouterConfig {
        topology,
        hub: args.hub.clone(),
        dlq_enabled: args.dlq,
    };
    log::info!("router config: {:?} (hub={:?}, dlq={})", topology.as_str(), args.hub, args.dlq);

    // Parse env vars
    let env: HashMap<String, String> = args
        .env
        .iter()
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    // Parse agent specifications: "id=path"
    let agent_specs: Vec<(String, String)> = args
        .agents
        .iter()
        .map(|spec| {
            let (id, path) = spec.split_once('=').unwrap_or((spec, spec));
            (id.to_string(), path.to_string())
        })
        .collect();

    let config = OrchestratorConfig {
        default_fuel: args.fuel,
        ..OrchestratorConfig::default()
    };

    let mut orch = Orchestrator::new(config)?;
    orch.configure_router(router_config);

    // Spawn all agents.
    // `--message` is only sent to the first agent (the leader).
    // Other agents receive `None` — they fall back to default role ("worker").
    // If per-agent messages are needed, extend `--agent` format to `id=path:message`.
    for (i, (id, wasm_path)) in agent_specs.iter().enumerate() {
        let init_msg = if i == 0 { args.message.as_deref() } else { None };
        println!("spawning agent '{id}' from {wasm_path} ...");
        match orch.spawn(
            id.clone(),
            wasm_path,
            Some(args.fuel),
            env.clone(),
            args.allow_url.clone(),
            init_msg,
        ) {
            Ok(Some(msg)) => {
                println!("  init response: {msg:?}");
            }
            Ok(None) => {
                println!("  init: no response");
            }
            Err(e) => {
                eprintln!("  error spawning '{id}': {e}");
            }
        }
    }

    // Configure auto-save if requested
    if let Some(interval) = args.save_every {
        orch.save_every(interval);
    }
    if let Some(dir) = &args.checkpoint_dir {
        let p = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&p)?;
        orch.set_checkpoint_dir(p);
    }

    // Run tick rounds
    for round in 1..=args.rounds {
        println!("\n--- Round {round} ---");
        let summary = orch.tick_all();

        for result in &summary.results {
            match &result.message {
                Some(msg) => {
                    println!(
                        "  [{}] {:?} (routed: {})",
                        result.agent_id, msg, result.messages_routed
                    );
                }
                None => {
                    println!(
                        "  [{}] no message (routed: {})",
                        result.agent_id, result.messages_routed
                    );
                }
            }
        }

        if summary.messages_routed > 0 && args.verbose {
            println!("  messages routed: {}", summary.messages_routed);
        }
        if summary.messages_dropped > 0 && args.verbose {
            println!("  messages dropped: {}", summary.messages_dropped);
        }
        if !summary.agent_inbox_depths.is_empty() && args.verbose {
            for (aid, depth) in &summary.agent_inbox_depths {
                println!("  {aid} inbox depth: {depth}");
            }
        }

        if !summary.crashed.is_empty() {
            println!("  CRASHED: {:?}", summary.crashed);
        }
    }

    // Print final status
    println!("\n--- Final Agent Status ---");
    for (id, status) in orch.list_agents() {
        let stats = orch.agent_stats(&id);
        if let Some((fuel, ticks, _)) = stats {
            println!("  {id}: {status:?} (fuel={fuel}, ticks={ticks})");
        } else {
            println!("  {id}: {status:?}");
        }
    }

    // Print observed messages (non-peer messages sent to orchestrator observer)
    if !orch.observed_messages.is_empty() && args.verbose {
        println!("\n--- Observed Messages ---");
        for (from, msg) in &orch.observed_messages {
            println!("  [{from}] {msg:?}");
        }
    }

    // Save full checkpoint at end if requested
    if args.full_snapshot {
        let dir = args.checkpoint_dir.unwrap_or_else(|| ".".to_string());
        let path = Path::new(&dir).join("checkpoint-final.json");
        orch.save_full(&path)?;
        println!("Full checkpoint saved to {}", path.display());
    }

    Ok(())
}

fn resume(args: ResumeArgs) -> anyhow::Result<()> {
    let log_level = if args.verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    env_logger::Builder::new()
        .filter_level(log_level)
        .init();

    println!("Loading checkpoint from {} ...", args.checkpoint);
    let mut orch = Orchestrator::load(Path::new(&args.checkpoint))?;

    let starting_round = orch.round_count();
    println!(
        "Resumed orchestrator with {} agents at round {}",
        orch.agent_count(),
        starting_round
    );

    // Configure auto-save if requested
    if let Some(interval) = args.save_every {
        orch.save_every(interval);
    }
    if let Some(dir) = &args.checkpoint_dir {
        let p = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&p)?;
        orch.set_checkpoint_dir(p);
    }

    // Run tick rounds
    for round in 1..=args.rounds {
        println!("\n--- Round {r} (resumed) ---", r = starting_round + round as usize);
        let summary = orch.tick_all();

        for result in &summary.results {
            match &result.message {
                Some(msg) => {
                    println!(
                        "  [{}] {:?} (routed: {})",
                        result.agent_id, msg, result.messages_routed
                    );
                }
                None => {
                    println!(
                        "  [{}] no message (routed: {})",
                        result.agent_id, result.messages_routed
                    );
                }
            }
        }

        if !summary.crashed.is_empty() {
            println!("  CRASHED: {:?}", summary.crashed);
        }
    }

    // Print final status
    println!("\n--- Final Agent Status ---");
    for (id, status) in orch.list_agents() {
        let stats = orch.agent_stats(&id);
        if let Some((fuel, ticks, _)) = stats {
            println!("  {id}: {status:?} (fuel={fuel}, ticks={ticks})");
        } else {
            println!("  {id}: {status:?}");
        }
    }

    // Save full checkpoint at end if requested
    if args.full_snapshot {
        let dir = args.checkpoint_dir.unwrap_or_else(|| ".".to_string());
        let path = Path::new(&dir).join("checkpoint-final.json");
        orch.save_full(&path)?;
        println!("Full checkpoint saved to {}", path.display());
    }

    Ok(())
}
