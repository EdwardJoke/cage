use std::collections::HashSet;

use super::MessageRouter;

/// Deliver every message to all agents except the sender.
///
/// The `to` parameter from `payload["to"]` is **ignored** — the topology
/// is the sole routing criterion.  This means an agent can send
/// `cage_peer_send("@all", …)` and every other agent will receive it
/// regardless of the payload's `to` field.
#[derive(Debug, Clone, Copy)]
pub struct BroadcastRouter;

impl MessageRouter for BroadcastRouter {
    fn resolve(&self, from: &str, _to: &str, agents: &HashSet<String>) -> Vec<String> {
        agents
            .iter()
            .filter(|id| *id != from)
            .cloned()
            .collect()
    }

    fn name(&self) -> &'static str {
        "broadcast"
    }
}
