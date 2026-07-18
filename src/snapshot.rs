#![allow(dead_code)]

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use wasmtime::*;
use wasmtime_wasi::WasiCtxBuilder;

use crate::ipc::AgentMessage;
use crate::orchestrator::{AgentInstance, AgentStatus, Orchestrator, OrchestratorConfig};
use crate::router::RouterConfig;
use crate::sandbox;

// ── Snapshot data structures ────────────────────────────────────────

/// Serializable snapshot of the full orchestrator state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorSnapshot {
    pub version: String,
    pub saved_at: String,
    pub round_count: usize,
    pub agents: Vec<AgentSnapshot>,
    pub router_config: RouterConfig,
    pub dlq: Vec<AgentMessage>,
    pub cumulative_stats: CumulativeStats,
}

/// Serializable state of a single agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub id: String,
    pub status: String,
    pub wasm_path: String,
    pub fuel_consumed: u64,
    pub fuel_remaining: u64,
    pub inbox: Vec<AgentMessage>,
    pub outbox: Vec<AgentMessage>,
    pub env: HashMap<String, String>,
    pub tick_count: u32,
    pub memory_state: Option<Vec<u8>>,
}

/// Aggregated observability counters across all rounds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CumulativeStats {
    pub total_messages_routed: usize,
    pub total_messages_dropped: usize,
    pub total_messages_dlq: usize,
    pub total_fuel_consumed: u64,
}

// ── SnapshotError ────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SnapshotError {
    Io(std::io::Error),
    Serde(serde_json::Error),
    WasmLoad(String),
    InvalidSnapshot(String),
    MemoryError(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Io(e) => write!(f, "I/O error: {e}"),
            SnapshotError::Serde(e) => write!(f, "serialization error: {e}"),
            SnapshotError::WasmLoad(e) => write!(f, "WASM load error: {e}"),
            SnapshotError::InvalidSnapshot(e) => write!(f, "invalid snapshot: {e}"),
            SnapshotError::MemoryError(e) => write!(f, "memory error: {e}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<std::io::Error> for SnapshotError {
    fn from(e: std::io::Error) -> Self {
        SnapshotError::Io(e)
    }
}

impl From<serde_json::Error> for SnapshotError {
    fn from(e: serde_json::Error) -> Self {
        SnapshotError::Serde(e)
    }
}

// ── Status conversion helpers ─────────────────────────────────────────

fn agent_status_to_string(status: &AgentStatus) -> String {
    match status {
        AgentStatus::Running => "Running".to_string(),
        AgentStatus::Paused => "Paused".to_string(),
        AgentStatus::Crashed(msg) => format!("Crashed({msg})"),
        AgentStatus::Terminated => "Terminated".to_string(),
    }
}

fn parse_agent_status(s: &str) -> AgentStatus {
    match s {
        "Running" => AgentStatus::Running,
        "Paused" => AgentStatus::Paused,
        "Terminated" => AgentStatus::Terminated,
        _ if s.starts_with("Crashed") => {
            let inner = s
                .strip_prefix("Crashed(")
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or(s)
                .to_string();
            AgentStatus::Crashed(inner)
        }
        _ => AgentStatus::Terminated,
    }
}

// ── Memory helpers ────────────────────────────────────────────────────

fn snapshot_memory(memory: &Memory, store: &Store<sandbox::SandboxState>) -> Vec<u8> {
    let size = memory.data_size(store);
    let mut buf = vec![0u8; size];
    if memory.read(store, 0, &mut buf).is_ok() {
        buf
    } else {
        Vec::new()
    }
}

fn restore_memory(
    memory: &Memory,
    store: &mut Store<sandbox::SandboxState>,
    bytes: &[u8],
) -> Result<(), wasmtime::MemoryAccessError> {
    memory.write(store, 0, bytes)
}

// ── Orchestrator persistence methods ─────────────────────────────────

impl Orchestrator {
    /// Convert current orchestrator state to a snapshot.
    pub fn to_snapshot(&self) -> OrchestratorSnapshot {
        let total_fuel: u64 = self
            .agents
            .values()
            .map(|inst| inst.fuel_consumed)
            .sum();

        let agents: Vec<AgentSnapshot> = self
            .agents
            .values()
            .map(|inst| {
                let state = inst.store.data();
                let fuel_remaining = inst.store.get_fuel().unwrap_or(0);
                AgentSnapshot {
                    id: inst.id.clone(),
                    status: agent_status_to_string(&inst.status),
                    wasm_path: inst.wasm_path.clone(),
                    fuel_consumed: inst.fuel_consumed,
                    fuel_remaining,
                    inbox: inst.inbox.iter().cloned().collect(),
                    outbox: state.outbox.iter().cloned().collect(),
                    env: state.env.clone(),
                    tick_count: inst.tick_count,
                    memory_state: None,
                }
            })
            .collect();

        let cumulative_stats = CumulativeStats {
            total_messages_routed: self.total_messages_routed,
            total_messages_dropped: self.total_messages_dropped,
            total_messages_dlq: self.total_messages_dlq,
            total_fuel_consumed: total_fuel,
        };

        OrchestratorSnapshot {
            version: "cage-snapshot-v1".to_string(),
            saved_at: Utc::now().to_rfc3339(),
            round_count: self.round_count,
            agents,
            router_config: self.router_config.clone(),
            dlq: self.dlq.iter().cloned().collect(),
            cumulative_stats,
        }
    }

    /// Reconstruct an orchestrator from a snapshot.
    pub fn from_snapshot(snap: OrchestratorSnapshot) -> Result<Self, SnapshotError> {
        if snap.version != "cage-snapshot-v1" {
            return Err(SnapshotError::InvalidSnapshot(format!(
                "unknown snapshot version '{}'",
                snap.version
            )));
        }

        let config = OrchestratorConfig::default();
        let mut orch = Orchestrator::new(config)
            .map_err(|e| SnapshotError::WasmLoad(e.to_string()))?;

        orch.configure_router(snap.router_config.clone());
        orch.round_count = snap.round_count;

        for agent_snap in &snap.agents {
            let wasm_bytes = std::fs::read(&agent_snap.wasm_path)
                .map_err(|e| {
                    SnapshotError::Io(std::io::Error::new(
                        e.kind(),
                        format!("failed to read WASM for '{}': {e}", agent_snap.id),
                    ))
                })?;

            let module = Module::new(&orch.engine, &wasm_bytes)
                .map_err(|e| SnapshotError::WasmLoad(e.to_string()))?;

            let wasi = WasiCtxBuilder::new().inherit_stdio().build_p1();
            let state = sandbox::SandboxState {
                wasi,
                agent_message: None,
                env: agent_snap.env.clone(),
                allowed_urls: Vec::new(),
                fuel_consumed: agent_snap.fuel_consumed,
                tick_count: 0,
                agent_id: agent_snap.id.clone(),
                outbox: VecDeque::from(agent_snap.outbox.clone()),
                inbox_state: None,
            };

            let mut store = Store::new(&orch.engine, state);
            store
                .set_fuel(agent_snap.fuel_remaining)
                .map_err(|e| SnapshotError::WasmLoad(e.to_string()))?;

            let mut linker = Linker::new(&orch.engine);
            wasmtime_wasi::p1::add_to_linker_sync(
                &mut linker,
                |state: &mut sandbox::SandboxState| &mut state.wasi,
            )
            .map_err(|e| SnapshotError::WasmLoad(e.to_string()))?;
            sandbox::register_host_functions(&mut linker)
                .map_err(|e| SnapshotError::WasmLoad(e.to_string()))?;

            let instance = linker
                .instantiate(&mut store, &module)
                .map_err(|e| SnapshotError::WasmLoad(e.to_string()))?;

            let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| {
                SnapshotError::InvalidSnapshot(format!(
                    "agent '{}' must export 'memory'",
                    agent_snap.id
                ))
            })?;

            if let Some(mem_bytes) = &agent_snap.memory_state {
                restore_memory(&memory, &mut store, mem_bytes)
                    .map_err(|e| SnapshotError::MemoryError(e.to_string()))?;
            }

            let status = parse_agent_status(&agent_snap.status);
            let id = agent_snap.id.clone();
            let inbox = VecDeque::from(agent_snap.inbox.clone());

            orch.agents.insert(
                id.clone(),
                AgentInstance {
                    id,
                    wasm_path: agent_snap.wasm_path.clone(),
                    store,
                    instance,
                    memory,
                    inbox,
                    fuel_budget: agent_snap.fuel_consumed + agent_snap.fuel_remaining,
                    fuel_consumed: agent_snap.fuel_consumed,
                    tick_count: agent_snap.tick_count,
                    status,
                },
            );
        }

        orch.dlq = VecDeque::from(snap.dlq);

        Ok(orch)
    }

    /// Save current state to a JSON file.
    pub fn save(&self, path: &Path) -> Result<(), SnapshotError> {
        let snapshot = self.to_snapshot();
        let json = serde_json::to_string_pretty(&snapshot)?;
        std::fs::write(path, json)?;
        log::info!("checkpoint saved to {}", path.display());
        Ok(())
    }

    /// Save with memory snapshots included (full fidelity).
    pub fn save_full(&self, path: &Path) -> Result<(), SnapshotError> {
        let total_fuel: u64 = self
            .agents
            .values()
            .map(|inst| inst.fuel_consumed)
            .sum();

        let agents: Vec<AgentSnapshot> = self
            .agents
            .values()
            .map(|inst| {
                let state = inst.store.data();
                let fuel_remaining = inst.store.get_fuel().unwrap_or(0);
                let mem = snapshot_memory(&inst.memory, &inst.store);
                AgentSnapshot {
                    id: inst.id.clone(),
                    status: agent_status_to_string(&inst.status),
                    wasm_path: inst.wasm_path.clone(),
                    fuel_consumed: inst.fuel_consumed,
                    fuel_remaining,
                    inbox: inst.inbox.iter().cloned().collect(),
                    outbox: state.outbox.iter().cloned().collect(),
                    env: state.env.clone(),
                    tick_count: inst.tick_count,
                    memory_state: Some(mem),
                }
            })
            .collect();

        let cumulative_stats = CumulativeStats {
            total_messages_routed: self.total_messages_routed,
            total_messages_dropped: self.total_messages_dropped,
            total_messages_dlq: self.total_messages_dlq,
            total_fuel_consumed: total_fuel,
        };

        let snapshot = OrchestratorSnapshot {
            version: "cage-snapshot-v1".to_string(),
            saved_at: Utc::now().to_rfc3339(),
            round_count: self.round_count,
            agents,
            router_config: self.router_config.clone(),
            dlq: self.dlq.iter().cloned().collect(),
            cumulative_stats,
        };

        let json = serde_json::to_string_pretty(&snapshot)?;
        std::fs::write(path, json)?;
        log::info!("full checkpoint saved to {}", path.display());
        Ok(())
    }

    /// Load state from a JSON file, reconstructing all agents.
    pub fn load(path: &Path) -> Result<Self, SnapshotError> {
        let json = std::fs::read_to_string(path)?;
        let snapshot: OrchestratorSnapshot = serde_json::from_str(&json)?;
        log::info!(
            "loading checkpoint from {} (round {}, {} agents)",
            path.display(),
            snapshot.round_count,
            snapshot.agents.len()
        );
        Self::from_snapshot(snapshot)
    }

    /// Set checkpoint interval in rounds.
    pub fn save_every(&mut self, n: usize) -> &mut Self {
        self.save_interval = Some(n);
        self
    }

    /// Set the directory for auto-save checkpoints.
    pub fn set_checkpoint_dir(&mut self, dir: PathBuf) -> &mut Self {
        self.checkpoint_dir = Some(dir);
        self
    }

    /// Export a human-readable summary (no binary memory data).
    pub fn export_summary(&self) -> Result<String, serde_json::Error> {
        let total_fuel: u64 = self.agents.values().map(|a| a.fuel_consumed).sum();
        let agent_list: Vec<serde_json::Value> = self
            .agents
            .values()
            .map(|inst| {
                let state = inst.store.data();
                let fuel_remaining = inst.store.get_fuel().unwrap_or(0);
                serde_json::json!({
                    "id": inst.id,
                    "status": agent_status_to_string(&inst.status),
                    "wasm_path": inst.wasm_path,
                    "fuel_consumed": inst.fuel_consumed,
                    "fuel_remaining": fuel_remaining,
                    "tick_count": inst.tick_count,
                    "inbox_depth": inst.inbox.len(),
                    "outbox_depth": state.outbox.len(),
                })
            })
            .collect();

        let summary = serde_json::json!({
            "version": "cage-snapshot-v1",
            "round_count": self.round_count,
            "agent_count": self.agents.len(),
            "agents": agent_list,
            "cumulative_stats": {
                "total_messages_routed": self.total_messages_routed,
                "total_messages_dropped": self.total_messages_dropped,
                "total_messages_dlq": self.total_messages_dlq,
                "total_fuel_consumed": total_fuel,
            },
            "dlq_depth": self.dlq.len(),
            "topology": self.router_config.topology.as_str(),
        });

        serde_json::to_string_pretty(&summary)
    }

}
