// Cage PyO3 bindings — Python access to the multi-agent WASM orchestrator.
//
// Usage:
//   from cage import Orchestrator
//   orch = Orchestrator()
//   orch.spawn("leader", "target/wasm32-wasip1/release/agent_p1.wasm")
//   summary = orch.tick_all()

use std::collections::HashMap;

use pyo3::exceptions::{PyKeyError, PyRuntimeError};
use pyo3::prelude::*;

use ::coplex_cage::orchestrator::AgentStatus;
use ::coplex_cage::orchestrator::{Orchestrator, OrchestratorConfig};
use ::coplex_cage::router::{RouterConfig, Topology};

// ── PyRoundSummary ──────────────────────────────────────────────────

/// Summary of one tick round across all running agents.
#[pyclass(name = "RoundSummary")]
#[derive(Debug, Clone)]
struct PyRoundSummary {
    /// Number of peer messages successfully routed this round.
    #[pyo3(get)]
    messages_routed: usize,
    /// Peer messages dropped because the target was not found.
    #[pyo3(get)]
    messages_dropped: usize,
    /// Number of unroutable messages moved to the Dead Letter Queue.
    #[pyo3(get)]
    messages_dlq: usize,
    /// Current depth of the Dead Letter Queue.
    #[pyo3(get)]
    dlq_depth: usize,
    /// Name of the active routing topology.
    #[pyo3(get)]
    routing_topology: String,
    /// Fuel consumed this round (delta, not cumulative).
    #[pyo3(get)]
    round_fuel: u64,
    /// Agent IDs that crashed this round.
    #[pyo3(get)]
    crashed: Vec<String>,
    /// Current inbox depth per agent (messages waiting to be delivered).
    #[pyo3(get)]
    agent_inbox_depths: HashMap<String, usize>,
    /// Non-peer messages observed during routing.
    #[pyo3(get)]
    observed_messages: Vec<String>,
}

impl PyRoundSummary {
    fn from_summary(
        summary: ::coplex_cage::orchestrator::RoundSummary,
        observed_messages: Vec<String>,
    ) -> Self {
        PyRoundSummary {
            messages_routed: summary.messages_routed,
            messages_dropped: summary.messages_dropped,
            messages_dlq: summary.messages_dlq,
            dlq_depth: summary.dlq_depth,
            routing_topology: summary.routing_topology,
            round_fuel: summary.round_fuel,
            crashed: summary.crashed,
            agent_inbox_depths: summary.agent_inbox_depths,
            observed_messages,
        }
    }
}

#[pymethods]
impl PyRoundSummary {
    fn __repr__(&self) -> String {
        format!(
            "RoundSummary(routed={}, dropped={}, dlq={}, dlq_depth={}, topology={}, fuel={}, crashed={})",
            self.messages_routed,
            self.messages_dropped,
            self.messages_dlq,
            self.dlq_depth,
            self.routing_topology,
            self.round_fuel,
            self.crashed.len(),
        )
    }
}

// ── PyOrchestrator ──────────────────────────────────────────────────

/// Multi-agent WASM orchestrator.
///
/// Loads WASM agents, drives tick rounds, and routes messages between them.
#[pyclass(name = "Orchestrator")]
struct PyOrchestrator {
    inner: Orchestrator,
}

#[pymethods]
impl PyOrchestrator {
    /// Create a new orchestrator with optional topology configuration.
    ///
    /// Args:
    ///     topology: Routing strategy — "direct" (default), "broadcast",
    ///               "pattern", or "hub-and-spoke".
    ///     hub: Agent ID of the hub (required when topology="hub-and-spoke").
    ///     dlq_enabled: If True, unroutable messages go to a Dead Letter Queue.
    #[new]
    #[pyo3(signature = (topology=None, hub=None, dlq_enabled=None))]
    fn new(topology: Option<String>, hub: Option<String>, dlq_enabled: Option<bool>) -> PyResult<Self> {
        let mut inner = Orchestrator::new(OrchestratorConfig::default())
            .map_err(|e| PyRuntimeError::new_err(format!("failed to create orchestrator: {e}")))?;

        // Apply topology if specified
        if topology.is_some() || hub.is_some() || dlq_enabled.is_some() {
            let topo = match topology.as_deref().unwrap_or("direct") {
                "direct" => Topology::Direct,
                "broadcast" => Topology::Broadcast,
                "pattern" => Topology::Pattern,
                "hub-and-spoke" => Topology::HubAndSpoke,
                other => {
                    return Err(PyRuntimeError::new_err(format!(
                        "unknown topology '{other}'; valid: direct, broadcast, pattern, hub-and-spoke"
                    )));
                }
            };
            let cfg = RouterConfig {
                topology: topo,
                hub,
                dlq_enabled: dlq_enabled.unwrap_or(false),
            };
            inner.configure_router(cfg);
        }

        Ok(PyOrchestrator { inner })
    }

    /// Load a WASM agent from `wasm_path` and register it as `agent_id`.
    ///
    /// Raises `RuntimeError` if the WASM file cannot be read or the agent
    /// fails to initialise.  Raises `KeyError` if `agent_id` already exists.
    fn spawn(&mut self, agent_id: String, wasm_path: String) -> PyResult<()> {
        self.inner
            .spawn(
                agent_id,
                &wasm_path,
                None,                         // default fuel
                HashMap::new(),               // no env
                Vec::new(),                   // no allowed URLs
                None,                         // no init payload
            )
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("already exists") {
                    PyKeyError::new_err(msg)
                } else {
                    PyRuntimeError::new_err(msg)
                }
            })?;
        Ok(())
    }

    /// Remove an agent.  Raises `KeyError` if not found.
    fn kill(&mut self, agent_id: String) -> PyResult<()> {
        self.inner.kill(&agent_id).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                PyKeyError::new_err(msg)
            } else {
                PyRuntimeError::new_err(msg)
            }
        })?;
        Ok(())
    }

    /// Pause an agent (it won't be ticked).  Raises `KeyError` if not found.
    fn pause(&mut self, agent_id: String) -> PyResult<()> {
        self.inner.pause(&agent_id).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                PyKeyError::new_err(msg)
            } else {
                PyRuntimeError::new_err(msg)
            }
        })
    }

    /// Resume a paused agent.  Raises `KeyError` if not found,
    /// `RuntimeError` if agent is not paused.
    fn resume(&mut self, agent_id: String) -> PyResult<()> {
        self.inner.resume(&agent_id).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                PyKeyError::new_err(msg)
            } else {
                PyRuntimeError::new_err(msg)
            }
        })
    }

    /// Run one tick cycle for a single agent, including inbox/outbox routing.
    ///
    /// Raises `KeyError` if the agent does not exist.
    fn tick_agent(&mut self, agent_id: String) -> PyResult<()> {
        self.inner.tick_agent(&agent_id).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") {
                PyKeyError::new_err(msg)
            } else {
                PyRuntimeError::new_err(msg)
            }
        })?;
        Ok(())
    }

    /// Tick all Running agents.  Returns a `RoundSummary` with routing stats.
    ///
    /// Releases the GIL during the tick so other Python threads can run.
    fn tick_all(&mut self, py: Python<'_>) -> PyResult<PyRoundSummary> {
        // Snapshot observed-message length before the tick so we can extract
        // only the *new* messages observed this round.
        let prev_obs_len = self.inner.observed_messages.len();

        // Release GIL during potentially long-running tick_all
        let summary = py.allow_threads(|| self.inner.tick_all());

        let observed: Vec<String> = self.inner.observed_messages[prev_obs_len..]
            .iter()
            .map(|(id, msg)| format!("[{id}] {}: {:?}", msg.kind, msg.payload))
            .collect();

        Ok(PyRoundSummary::from_summary(summary, observed))
    }

    /// Return the status string for an agent.  Raises `KeyError` if not found.
    fn agent_status(&self, agent_id: String) -> PyResult<String> {
        match self.inner.agent_stats(&agent_id) {
            Some((_, _, status)) => {
                let s = match status {
                    AgentStatus::Running => "Running",
                    AgentStatus::Paused => "Paused",
                    AgentStatus::Crashed(_) => "Crashed",
                    AgentStatus::Terminated => "Terminated",
                };
                Ok(s.to_string())
            }
            None => Err(PyKeyError::new_err(format!("Agent '{agent_id}' not found"))),
        }
    }

    /// Number of agents currently registered.
    fn agent_count(&self) -> usize {
        self.inner.agent_count()
    }

    /// List all agents as `(id, status)` tuples.
    fn list_agents(&self) -> Vec<(String, String)> {
        self.inner
            .list_agents()
            .into_iter()
            .map(|(id, status)| {
                let s = match status {
                    AgentStatus::Running => "Running",
                    AgentStatus::Paused => "Paused",
                    AgentStatus::Crashed(_) => "Crashed",
                    AgentStatus::Terminated => "Terminated",
                };
                (id, s.to_string())
            })
            .collect()
    }

    /// Save orchestrator state to a checkpoint JSON file.
    fn save(&self, path: String) -> PyResult<()> {
        self.inner
            .save(std::path::Path::new(&path))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Save orchestrator state with WASM memory snapshots.
    fn save_full(&self, path: String) -> PyResult<()> {
        self.inner
            .save_full(std::path::Path::new(&path))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Load orchestrator state from a checkpoint JSON file.
    #[staticmethod]
    fn load(path: String) -> PyResult<Self> {
        let inner = Orchestrator::load(std::path::Path::new(&path))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(PyOrchestrator { inner })
    }

    /// Export a human-readable summary of the orchestrator state.
    fn export_summary(&self) -> PyResult<String> {
        self.inner
            .export_summary()
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Set checkpoint save interval in rounds.
    fn set_save_every(&mut self, n: usize) {
        self.inner.save_every(n);
    }

    /// Set the directory for auto-save checkpoints.
    fn set_checkpoint_dir(&mut self, dir: String) {
        self.inner.set_checkpoint_dir(std::path::PathBuf::from(dir));
    }

    fn __repr__(&self) -> String {
        format!("Orchestrator(agents={})", self.inner.agent_count())
    }
}

// ── Module ──────────────────────────────────────────────────────────

/// Cage: multi-agent WASM orchestrator.
#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyOrchestrator>()?;
    m.add_class::<PyRoundSummary>()?;
    Ok(())
}
