// Cage Phase 3 — MessageRouter Topology
//
// Pluggable routing strategies for inter-agent messages.
// The trait is used by Orchestrator::route() and can be swapped at runtime.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

mod broadcast;
mod direct;
mod hub_and_spoke;
mod pattern;

pub use broadcast::BroadcastRouter;
pub use direct::DirectRouter;
pub use hub_and_spoke::HubAndSpokeRouter;
pub use pattern::PatternRouter;

// ── Topology ────────────────────────────────────────────────────────

/// Enum of supported routing topologies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Topology {
    /// Route by exact agent ID match (`payload["to"]` → one agent).
    Direct,
    /// Deliver to every agent *except* the sender.
    Broadcast,
    /// Match agent IDs by glob/wildcard pattern.
    Pattern,
    /// All non-hub traffic routes to the hub; hub forwards to targets.
    HubAndSpoke,
}

impl Topology {
    /// Build a `Box<dyn MessageRouter>` for this topology.
    pub fn build_router(&self, hub: Option<String>) -> Box<dyn MessageRouter> {
        match self {
            Topology::Direct => Box::new(DirectRouter),
            Topology::Broadcast => Box::new(BroadcastRouter),
            Topology::Pattern => Box::new(PatternRouter),
            Topology::HubAndSpoke => Box::new(HubAndSpokeRouter {
                hub: hub.expect("HubAndSpoke requires --hub"),
            }),
        }
    }

    /// Return a stable string name (useful for CLI parsing and RoundSummary).
    pub fn as_str(&self) -> &'static str {
        match self {
            Topology::Direct => "direct",
            Topology::Broadcast => "broadcast",
            Topology::Pattern => "pattern",
            Topology::HubAndSpoke => "hub-and-spoke",
        }
    }
}

// ── RouterConfig ────────────────────────────────────────────────────

/// Configuration for the message router.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    /// Which topology to use.
    pub topology: Topology,
    /// Hub agent ID (required for HubAndSpoke, ignored otherwise).
    pub hub: Option<String>,
    /// When `true`, unroutable messages go to a Dead Letter Queue instead
    /// of being silently dropped.
    pub dlq_enabled: bool,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            topology: Topology::Direct,
            hub: None,
            dlq_enabled: false,
        }
    }
}

// ── MessageRouter Trait ─────────────────────────────────────────────

/// A pluggable routing strategy that resolves a destination string to a
/// set of target agent IDs.
pub trait MessageRouter: Send + Sync {
    /// Given the sender, the destination string from `payload["to"]`, and
    /// the set of all known agent IDs, return the list of target agents
    /// that should receive this message.
    ///
    /// Return an empty `Vec` to signal that the message is unroutable.
    fn resolve(&self, from: &str, to: &str, agents: &HashSet<String>) -> Vec<String>;

    /// Human-readable name of this router (e.g. `"direct"`, `"broadcast"`).
    fn name(&self) -> &'static str;
}
