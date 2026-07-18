// Cage orchestrator — multi-agent lifecycle manager
//
// Phase 2: inter-agent messaging via inbox/outbox + route().
// Phase 3: pluggable MessageRouter topology + Dead Letter Queue.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use wasmtime::*;
use wasmtime_wasi::WasiCtxBuilder;

use crate::ipc::AgentMessage;
use crate::router::{MessageRouter, RouterConfig};
use crate::sandbox;

pub(crate) type SandboxState = sandbox::SandboxState;
pub(crate) type InboxState = sandbox::InboxState;

// ── Public types ────────────────────────────────────────────────────

pub type AgentId = String;

/// Status of an agent in the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    /// Agent loaded, init called, ready for ticks.
    Running,
    /// Agent explicitly paused by the host.
    Paused,
    /// Agent exited with a trap or runtime error.
    Crashed(String),
    /// Agent was killed / removed.
    Terminated,
}

/// Configuration for the orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    /// Default fuel budget per agent (overridable per spawn).
    pub default_fuel: u64,
    /// Maximum number of agents allowed.
    pub max_agents: usize,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            default_fuel: 500_000,
            max_agents: 64,
        }
    }
}

/// Result of a single agent tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickResult {
    pub agent_id: AgentId,
    pub message: Option<AgentMessage>,
    /// Number of peer messages routed from this agent's outbox this tick.
    pub messages_routed: usize,
    /// Number of peer messages dropped (target not found, etc.).
    pub messages_dropped: usize,
}

/// Summary of one round across all running agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundSummary {
    pub results: Vec<TickResult>,
    /// Fuel consumed *this round only* (delta, not cumulative).
    pub round_fuel: u64,
    pub crashed: Vec<AgentId>,
    /// Total peer messages successfully routed this round.
    pub messages_routed: usize,
    /// Peer messages dropped because target was not found (includes DLQ).
    pub messages_dropped: usize,
    /// Number of unroutable messages moved to the Dead Letter Queue this round.
    pub messages_dlq: usize,
    /// Current depth of the Dead Letter Queue.
    pub dlq_depth: usize,
    /// Name of the routing topology active this round.
    pub routing_topology: String,
    /// Current inbox depth per agent (messages waiting to be delivered).
    pub agent_inbox_depths: HashMap<AgentId, usize>,
}

// ── Per-agent runtime state ─────────────────────────────────────────

pub(crate) struct AgentInstance {
    pub(crate) id: AgentId,
    pub(crate) wasm_path: String,
    pub(crate) store: Store<SandboxState>,
    pub(crate) instance: Instance,
    pub(crate) memory: Memory,
    /// Messages from other agents queued for delivery before next tick.
    pub(crate) inbox: VecDeque<AgentMessage>,
    #[allow(dead_code)]
    pub(crate) fuel_budget: u64,
    pub(crate) fuel_consumed: u64,
    pub(crate) tick_count: u32,
    pub(crate) status: AgentStatus,
}

// ── Orchestrator ────────────────────────────────────────────────────

pub struct Orchestrator {
    pub(crate) engine: Engine,
    pub(crate) config: OrchestratorConfig,
    pub(crate) agents: HashMap<AgentId, AgentInstance>,
    /// Accumulated observed messages across rounds.
    pub observed_messages: Vec<(AgentId, AgentMessage)>,
    /// Pluggable message router.
    pub(crate) router: Box<dyn MessageRouter>,
    /// Router configuration (topology, hub, DLQ toggle).
    pub(crate) router_config: RouterConfig,
    /// Dead Letter Queue — unroutable messages when DLQ is enabled.
    pub(crate) dlq: VecDeque<AgentMessage>,
    /// Count of messages added to DLQ this round (reset each tick_all).
    pub(crate) messages_dlq: usize,
    /// Number of completed tick rounds.
    pub(crate) round_count: usize,
    /// Cumulative messages routed across all rounds.
    pub(crate) total_messages_routed: usize,
    /// Cumulative messages dropped across all rounds.
    pub(crate) total_messages_dropped: usize,
    /// Cumulative messages sent to DLQ across all rounds.
    pub(crate) total_messages_dlq: usize,
    /// Checkpoint interval (None = disabled).
    pub(crate) save_interval: Option<usize>,
    /// Directory for auto-save checkpoints.
    pub(crate) checkpoint_dir: Option<std::path::PathBuf>,
}

impl Orchestrator {
    /// Create a new orchestrator with the given configuration.
    pub fn new(config: OrchestratorConfig) -> Result<Self> {
        let mut engine_cfg = Config::new();
        engine_cfg.consume_fuel(true);
        engine_cfg.wasm_multi_value(true);
        engine_cfg.wasm_bulk_memory(true);

        let engine = Engine::new(&engine_cfg)?;

        let router_config = RouterConfig::default();
        let router = router_config.topology.build_router(None);

        Ok(Self {
            engine,
            config,
            agents: HashMap::new(),
            observed_messages: Vec::new(),
            router,
            router_config,
            dlq: VecDeque::new(),
            messages_dlq: 0,
            round_count: 0,
            total_messages_routed: 0,
            total_messages_dropped: 0,
            total_messages_dlq: 0,
            save_interval: None,
            checkpoint_dir: None,
        })
    }

    /// Number of agents currently registered (includes all statuses).
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Return a list of (agent_id, status) for all agents.
    pub fn list_agents(&self) -> Vec<(AgentId, &AgentStatus)> {
        self.agents
            .iter()
            .map(|(id, inst)| (id.clone(), &inst.status))
            .collect()
    }

    /// Return stats for a specific agent.
    pub fn agent_stats(&self, id: &AgentId) -> Option<(u64, u32, &AgentStatus)> {
        self.agents
            .get(id)
            .map(|inst| (inst.fuel_consumed, inst.tick_count, &inst.status))
    }

    // ── Router configuration ─────────────────────

    /// Replace the message router with a new configuration.
    /// Agents already spawned will use the new router from the next tick.
    pub fn configure_router(&mut self, config: RouterConfig) {
        let hub = config.hub.clone();
        self.router = config.topology.build_router(hub);
        self.router_config = config;
    }

    /// Return a reference to the current router configuration.
    pub fn router_config(&self) -> &RouterConfig {
        &self.router_config
    }

    /// Return the current Dead Letter Queue contents (for inspection).
    pub fn dlq(&self) -> &VecDeque<AgentMessage> {
        &self.dlq
    }

    /// Drain and return the current DLQ (e.g. after processing in Python).
    pub fn drain_dlq(&mut self) -> Vec<AgentMessage> {
        self.dlq.drain(..).collect()
    }

    // ── Spawn ───────────────────────────────────────────────────

    /// Load a WASM agent, call `_cage_init`, and register it.
    pub fn spawn(
        &mut self,
        id: AgentId,
        wasm_path: &str,
        fuel: Option<u64>,
        env: HashMap<String, String>,
        allowed_urls: Vec<String>,
        init_payload: Option<&str>,
    ) -> Result<Option<AgentMessage>> {
        if self.agents.contains_key(&id) {
            anyhow::bail!("agent '{id}' already exists");
        }
        if self.agents.len() >= self.config.max_agents {
            anyhow::bail!("max agents ({}) reached", self.config.max_agents);
        }

        let fuel_budget = fuel.unwrap_or(self.config.default_fuel);
        let module = Module::from_file(&self.engine, wasm_path)?;

        let wasi = WasiCtxBuilder::new().inherit_stdio().build_p1();
        let state = SandboxState {
            wasi,
            agent_message: None,
            env: env.clone(),
            allowed_urls: allowed_urls.clone(),
            fuel_consumed: 0,
            tick_count: 0,
            agent_id: id.clone(),
            outbox: VecDeque::new(),
            inbox_state: None,
        };

        let mut store = Store::new(&self.engine, state);
        store.set_fuel(fuel_budget)?;

        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::p1::add_to_linker_sync(
            &mut linker,
            |state: &mut SandboxState| &mut state.wasi,
        )?;
        sandbox::register_host_functions(&mut linker)?;

        let instance = linker.instantiate(&mut store, &module)?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow!("agent '{id}' must export linear memory as \"memory\""))?;

        // ── Call _cage_init ──────────────────────────────────────────
        let (ptr, len) = if let Some(payload) = init_payload {
            let msg_bytes = payload.as_bytes();
            let size = msg_bytes.len() as i32;

            let alloc = instance.get_typed_func::<i32, i32>(&mut store, "_cage_alloc")?;
            let ptr = alloc.call(&mut store, size)?;

            if ptr < 0 {
                anyhow::bail!("_cage_alloc returned negative pointer: {ptr}");
            }
            memory.write(&mut store, ptr as usize, msg_bytes)?;
            (ptr, size)
        } else {
            (0, 0)
        };

        let init_result =
            match instance.get_typed_func::<(i32, i32), i32>(&mut store, "_cage_init") {
                Ok(init) => {
                    let result = init.call(&mut store, (ptr, len))?;
                    if result != 0 {
                        anyhow::bail!("_cage_init returned non-zero: {result}");
                    }
                    log::info!("agent '{id}' initialized successfully");
                    store.data_mut().agent_message.take()
                }
                Err(_) => {
                    log::info!("agent '{id}' does not export `_cage_init`, skipping");
                    None
                }
            };

        if ptr != 0
            && let Ok(free) = instance.get_typed_func::<i32, ()>(&mut store, "_cage_free")
        {
            free.call(&mut store, ptr)?;
        }

        let fuel_after = store.get_fuel()?;
        let fuel_consumed = fuel_budget.saturating_sub(fuel_after);
        store.data_mut().fuel_consumed = fuel_consumed;

        self.agents.insert(
            id.clone(),
            AgentInstance {
                id,
                wasm_path: wasm_path.to_string(),
                store,
                instance,
                memory,
                inbox: VecDeque::new(),
                fuel_budget,
                fuel_consumed,
                tick_count: 0,
                status: AgentStatus::Running,
            },
        );

        Ok(init_result)
    }

    // ── Tick ────────────────────────────────────────────────────
    //
    //  Before tick: deliver inbox messages to agent memory
    //  During tick: agent may call cage_peer_send → queued to outbox
    //  After tick:  drain outbox, route to targets

    /// Execute one tick for the named agent with inbox/outbox routing.
    pub fn tick_agent(&mut self, id: &AgentId) -> Result<TickResult> {
        let inst = self
            .agents
            .get_mut(id)
            .ok_or_else(|| anyhow!("agent '{id}' not found"))?;

        if inst.status != AgentStatus::Running {
            return Ok(TickResult {
                agent_id: id.clone(),
                message: None,
                messages_routed: 0,
                messages_dropped: 0,
            });
        }

        // ── Phase 2a: deliver inbox to agent memory before tick ─────
        Self::deliver_inbox(inst)?;

        let fuel_before = inst.store.get_fuel()?;

        // ── Execute _cage_tick ──────────────────────────────────────
        let tick_result_msg = match inst
            .instance
            .get_typed_func::<(), i32>(&mut inst.store, "_cage_tick")
        {
            Ok(tick) => {
                let r = tick.call(&mut inst.store, ());
                match r {
                    Ok(0) => Ok(inst.store.data_mut().agent_message.take()),
                    Ok(nz) => Err(anyhow!("_cage_tick returned non-zero: {nz}")),
                    Err(trap) => {
                        let msg = format!("{trap:#}");
                        inst.status = AgentStatus::Crashed(msg.clone());
                        Err(anyhow!("agent '{id}' crashed: {msg}"))
                    }
                }
            }
            Err(_) => {
                log::info!("agent '{id}' does not export `_cage_tick`");
                Ok(None)
            }
        };

        let fuel_after = inst.store.get_fuel()?;
        let consumed = fuel_before.saturating_sub(fuel_after);
        inst.fuel_consumed += consumed;
        inst.tick_count += 1;

        log::info!(
            "agent '{id}' consumed {consumed} fuel during tick ({} total, {} ticks)",
            inst.fuel_consumed,
            inst.tick_count,
        );

        // Mark as crashed if fuel exhausted mid-execution
        if fuel_after == 0 && matches!(inst.status, AgentStatus::Running) {
            inst.status = AgentStatus::Crashed("out of fuel".to_string());
        }

        // ── Phase 2b: drain outbox ──────────────────────────────────
        // Collect outbox + clone id, then drop the mutable borrow on
        // self.agents BEFORE calling self.route() to avoid borrow conflict.
        let outbox: Vec<AgentMessage> =
            inst.store.data_mut().outbox.drain(..).collect();
        let messages_routed = outbox.len();
        let agent_id = inst.id.clone();
        // Release the mutable borrow from self.agents before calling self.route()
        // (use let _ = inst to suppress drop-on-ref warning)
        let _ = inst;

        // Route each outbound message to its target
        let mut messages_dropped = 0usize;
        for msg in outbox {
            if let Err(e) = self.route(&agent_id, msg) {
                log::warn!("route from '{agent_id}' failed: {e}");
                messages_dropped += 1;
            }
        }

        let msg = tick_result_msg.unwrap_or(None);
        Ok(TickResult {
            agent_id: id.clone(),
            message: msg,
            messages_routed,
            messages_dropped,
        })
    }

    /// Write the first inbox message into agent linear memory so the
    /// agent can read it via `cage_inbox_read()` during its tick.
    fn deliver_inbox(inst: &mut AgentInstance) -> Result<()> {
        let msg = match inst.inbox.pop_front() {
            Some(m) => m,
            None => {
                // No pending messages — clear any stale inbox state
                inst.store.data_mut().inbox_state = None;
                return Ok(());
            }
        };

        // Serialise to JSON
        let json_bytes = serde_json::to_vec(&msg)
            .map_err(|e| anyhow!("inbox serialization: {e}"))?;
        let size = json_bytes.len() as i32;

        // Allocate in agent's linear memory
        let alloc = match inst
            .instance
            .get_typed_func::<i32, i32>(&mut inst.store, "_cage_alloc")
        {
            Ok(f) => f,
            Err(_) => {
                anyhow::bail!("agent '{}' has no _cage_alloc", inst.id);
            }
        };
        let ptr = alloc.call(&mut inst.store, size)?;
        if ptr < 0 {
            anyhow::bail!("_cage_alloc returned {ptr}");
        }

        // Write the serialised message
        inst.memory
            .write(&mut inst.store, ptr as usize, &json_bytes)?;

        // Record the pointer + length in SandboxState so cage_inbox_read can find it
        inst.store.data_mut().inbox_state = Some(InboxState { ptr, len: size });

        Ok(())
    }

    /// Route a peer message from `from` to its target(s) using the
    /// active `MessageRouter`.
    fn route(&mut self, from: &AgentId, msg: AgentMessage) -> Result<()> {
        // Extract destination string BEFORE matching on msg (avoids borrow-on-move)
        let to = msg.payload["to"]
            .as_str()
            .unwrap_or("")
            .to_string();

        if to.is_empty() {
            self.observed_messages.push((from.clone(), msg));
            return Err(anyhow!("peer message missing 'to' field"));
        }

        let agent_ids: HashSet<String> = self.agents.keys().cloned().collect();
        let targets = self.router.resolve(from, &to, &agent_ids);

        if targets.is_empty() {
            // Unroutable: DLQ or drop
            if self.router_config.dlq_enabled {
                self.dlq.push_back(msg);
                self.messages_dlq += 1;
            } else {
                self.observed_messages.push((from.clone(), msg));
            }
            return Err(anyhow!("target '{to}' not found (topology: {})", self.router.name()));
        }

        for target in &targets {
            if let Some(target_inst) = self.agents.get_mut(target) {
                target_inst.inbox.push_back(msg.clone());
                log::info!("route: {from} -> {target}");
            } else if self.router_config.dlq_enabled {
                self.dlq.push_back(msg.clone());
                self.messages_dlq += 1;
            } else {
                self.observed_messages.push((from.clone(), msg.clone()));
            }
        }

        Ok(())
    }

    /// Tick all Running agents.  Returns a `RoundSummary`.
    pub fn tick_all(&mut self) -> RoundSummary {
        let ids: Vec<AgentId> = self
            .agents
            .iter()
            .filter(|(_, inst)| inst.status == AgentStatus::Running)
            .map(|(id, _)| id.clone())
            .collect();

        let mut results = Vec::new();
        let mut round_fuel = 0u64;
        let mut crashed = Vec::new();
        let mut messages_routed = 0usize;
        let mut messages_dropped = 0usize;

        for id in ids {
            let fuel_before = self
                .agents
                .get(&id)
                .map(|inst| inst.fuel_consumed)
                .unwrap_or(0);

            match self.tick_agent(&id) {
                Ok(tick_result) => {
                    messages_routed += tick_result.messages_routed;
                    messages_dropped += tick_result.messages_dropped;

                    // If the tick crashed but still returned Ok, check status
                    if let Some(inst) = self.agents.get(&id)
                        && matches!(inst.status, AgentStatus::Crashed(_))
                    {
                        crashed.push(id.clone());
                    }

                    results.push(tick_result);
                    let fuel_after = self
                        .agents
                        .get(&id)
                        .map(|inst| inst.fuel_consumed)
                        .unwrap_or(fuel_before);
                    round_fuel += fuel_after.saturating_sub(fuel_before);
                }
                Err(e) => {
                    log::warn!("tick_agent('{id}') error: {e}");
                    crashed.push(id.clone());
                    results.push(TickResult {
                        agent_id: id.clone(),
                        message: None,
                        messages_routed: 0,
                        messages_dropped: 0,
                    });
                }
            }
        }

        // Collect inbox depths
        let mut agent_inbox_depths = HashMap::new();
        for (aid, inst) in &self.agents {
            let depth = inst.inbox.len();
            if depth > 0 {
                agent_inbox_depths.insert(aid.clone(), depth);
            }
        }

        let messages_dlq = self.messages_dlq;
        self.messages_dlq = 0; // reset round counter

        self.round_count += 1;
        self.total_messages_routed += messages_routed;
        self.total_messages_dropped += messages_dropped;
        self.total_messages_dlq += messages_dlq;

        // Auto-save if configured
        if let Some(interval) = self.save_interval
            && self.round_count.is_multiple_of(interval)
            && let Some(dir) = &self.checkpoint_dir
        {
            let path = dir.join(format!("checkpoint-{}.json", self.round_count));
            if let Err(e) = self.save(&path) {
                log::warn!("auto-save at round {} failed: {e}", self.round_count);
            }
        }

        RoundSummary {
            results,
            round_fuel,
            crashed,
            messages_routed,
            messages_dropped,
            messages_dlq,
            dlq_depth: self.dlq.len(),
            routing_topology: self.router.name().to_string(),
            agent_inbox_depths,
        }
    }

    // ── Lifecycle management ────────────────────────────────────

    /// Remove an agent from the registry.  Returns its final stats.
    pub fn kill(&mut self, id: &AgentId) -> Result<(u64, u32)> {
        match self.agents.remove(id) {
            Some(inst) => {
                log::info!(
                    "agent '{id}' killed (fuel={}, ticks={})",
                    inst.fuel_consumed,
                    inst.tick_count
                );
                Ok((inst.fuel_consumed, inst.tick_count))
            }
            None => Err(anyhow!("agent '{id}' not found")),
        }
    }

    /// Pause an agent (it will not be ticked in future rounds).
    pub fn pause(&mut self, id: &AgentId) -> Result<()> {
        let inst = self
            .agents
            .get_mut(id)
            .ok_or_else(|| anyhow!("agent '{id}' not found"))?;
        inst.status = AgentStatus::Paused;
        Ok(())
    }

    /// Resume a paused agent.
    pub fn resume(&mut self, id: &AgentId) -> Result<()> {
        let inst = self
            .agents
            .get_mut(id)
            .ok_or_else(|| anyhow!("agent '{id}' not found"))?;
        if inst.status != AgentStatus::Paused {
            anyhow::bail!("agent '{id}' is not paused (status: {:?})", inst.status);
        }
        inst.status = AgentStatus::Running;
        Ok(())
    }

    /// Number of completed tick rounds.
    pub fn round_count(&self) -> usize {
        self.round_count
    }

    /// Cumulative total messages routed across all rounds.
    pub fn total_messages_routed(&self) -> usize {
        self.total_messages_routed
    }

    /// Cumulative total messages dropped across all rounds.
    pub fn total_messages_dropped(&self) -> usize {
        self.total_messages_dropped
    }

    /// Cumulative total messages sent to DLQ across all rounds.
    pub fn total_messages_dlq(&self) -> usize {
        self.total_messages_dlq
    }
}

impl Drop for Orchestrator {
    fn drop(&mut self) {
        log::info!(
            "orchestrator shutting down ({} agents remaining)",
            self.agents.len()
        );
    }
}
