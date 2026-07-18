#![allow(dead_code)]

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64;
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
    pub allowed_urls: Vec<String>,
    pub tick_count: u32,
    #[serde(with = "base64_memory", default)]
    pub memory_state: Option<Vec<u8>>,
}

/// Serialize agent linear memory as a single compact base64 string rather than
/// a JSON array of bytes, which would emit one line per byte for multi-MB
/// memories.
mod base64_memory {
    use super::BASE64;
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => serializer.serialize_some(&BASE64.encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        let encoded: Option<String> = Option::deserialize(deserializer)?;
        match encoded {
            Some(s) => BASE64
                .decode(s.as_bytes())
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
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

/// Size of a WebAssembly linear-memory page in bytes.
const WASM_PAGE_SIZE: usize = 64 * 1024;

fn restore_memory(
    memory: &Memory,
    store: &mut Store<sandbox::SandboxState>,
    bytes: &[u8],
) -> Result<(), SnapshotError> {
    // A freshly instantiated module only has its declared minimum number of
    // pages. If the agent had grown its memory before the checkpoint, the
    // snapshot buffer is larger than the current memory, so grow it to fit
    // before writing to avoid an out-of-bounds write error.
    let current = memory.data_size(&mut *store);
    if bytes.len() > current {
        let missing = bytes.len() - current;
        let extra_pages = missing.div_ceil(WASM_PAGE_SIZE) as u64;
        memory
            .grow(&mut *store, extra_pages)
            .map_err(|e| SnapshotError::MemoryError(e.to_string()))?;
    }
    memory
        .write(store, 0, bytes)
        .map_err(|e| SnapshotError::MemoryError(e.to_string()))
}

// ── WASM path validation ──────────────────────────────────────────────

/// Validate the WASM path embedded in a checkpoint before reading it.
///
/// The `wasm_path` stored in a checkpoint is fully controlled by whoever wrote
/// the file. A checkpoint from an untrusted source could therefore point at an
/// arbitrary location on the filesystem. Callers are responsible for only
/// resuming from trusted checkpoints, but as defense in depth we reject paths
/// that are empty or resolve to anything other than an existing regular file
/// (e.g. symlinks, directories, or device nodes).
fn validate_wasm_path(wasm_path: &str, agent_id: &str) -> Result<(), SnapshotError> {
    if wasm_path.is_empty() {
        return Err(SnapshotError::InvalidSnapshot(format!(
            "agent '{agent_id}' has an empty WASM path"
        )));
    }

    let meta = std::fs::symlink_metadata(wasm_path).map_err(|e| {
        SnapshotError::Io(std::io::Error::new(
            e.kind(),
            format!("failed to stat WASM for '{agent_id}' at '{wasm_path}': {e}"),
        ))
    })?;

    if !meta.file_type().is_file() {
        return Err(SnapshotError::InvalidSnapshot(format!(
            "agent '{agent_id}' WASM path '{wasm_path}' is not a regular file"
        )));
    }

    log::warn!(
        "loading WASM for agent '{agent_id}' from checkpoint-specified path '{wasm_path}'; \
         only resume from trusted checkpoints"
    );

    Ok(())
}

// ── Orchestrator persistence methods ─────────────────────────────────

impl Orchestrator {
    /// Convert current orchestrator state to a snapshot.
    pub fn to_snapshot(&self) -> OrchestratorSnapshot {
        let total_fuel: u64 = self.agents.values().map(|inst| inst.fuel_consumed).sum();

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
                    allowed_urls: state.allowed_urls.clone(),
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
        let mut orch =
            Orchestrator::new(config).map_err(|e| SnapshotError::WasmLoad(e.to_string()))?;

        orch.configure_router(snap.router_config.clone());
        orch.round_count = snap.round_count;

        for agent_snap in &snap.agents {
            validate_wasm_path(&agent_snap.wasm_path, &agent_snap.id)?;

            let wasm_bytes = std::fs::read(&agent_snap.wasm_path).map_err(|e| {
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
                allowed_urls: agent_snap.allowed_urls.clone(),
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
                restore_memory(&memory, &mut store, mem_bytes)?;
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
        orch.total_messages_routed = snap.cumulative_stats.total_messages_routed;
        orch.total_messages_dropped = snap.cumulative_stats.total_messages_dropped;
        orch.total_messages_dlq = snap.cumulative_stats.total_messages_dlq;

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
        let total_fuel: u64 = self.agents.values().map(|inst| inst.fuel_consumed).sum();

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
                    allowed_urls: state.allowed_urls.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent(memory_state: Option<Vec<u8>>) -> AgentSnapshot {
        AgentSnapshot {
            id: "agent-a".to_string(),
            status: "Running".to_string(),
            wasm_path: "agent.wasm".to_string(),
            fuel_consumed: 42,
            fuel_remaining: 100,
            inbox: Vec::new(),
            outbox: Vec::new(),
            env: HashMap::new(),
            allowed_urls: vec!["https://example.com".to_string()],
            tick_count: 3,
            memory_state,
        }
    }

    #[test]
    fn agent_snapshot_round_trips_allowed_urls_and_memory() {
        let agent = sample_agent(Some(vec![1, 2, 3, 4, 5]));
        let json = serde_json::to_string(&agent).unwrap();
        let back: AgentSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.allowed_urls, vec!["https://example.com".to_string()]);
        assert_eq!(back.memory_state, Some(vec![1, 2, 3, 4, 5]));
    }

    #[test]
    fn memory_state_serializes_as_compact_base64_string() {
        let agent = sample_agent(Some(vec![0u8; 4096]));
        let json = serde_json::to_string_pretty(&agent).unwrap();
        // A base64 string stays on a single line rather than emitting one
        // array element per byte.
        assert!(json.lines().count() < 100, "memory dumped as byte array");
        assert!(json.contains("\"memory_state\": \""));
    }

    #[test]
    fn absent_memory_state_serializes_as_null() {
        let agent = sample_agent(None);
        let json = serde_json::to_string(&agent).unwrap();
        assert!(json.contains("\"memory_state\":null"));
        let back: AgentSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.memory_state, None);
    }

    #[test]
    fn validate_wasm_path_rejects_empty_and_missing() {
        assert!(validate_wasm_path("", "agent-a").is_err());
        assert!(validate_wasm_path("/nonexistent/path/agent.wasm", "agent-a").is_err());
    }

    #[test]
    fn validate_wasm_path_rejects_directory_and_accepts_file() {
        let dir = std::env::temp_dir().join(format!("cage-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(validate_wasm_path(dir.to_str().unwrap(), "agent-a").is_err());

        let file = dir.join("agent.wasm");
        std::fs::write(&file, b"\0asm").unwrap();
        assert!(validate_wasm_path(file.to_str().unwrap(), "agent-a").is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }
}
