use std::collections::HashSet;

use super::MessageRouter;

/// Match agent IDs by glob/wildcard pattern.
///
/// Uses the `glob::Pattern` crate to match `payload["to"]` against all
/// registered agent IDs.
///
/// # Examples
///
/// An agent sending `cage_peer_send("worker-*", …)` in a round with agents
/// `["leader", "worker-a", "worker-b", "other"]` will route the message
/// to `worker-a` and `worker-b`.
#[derive(Debug, Clone, Copy)]
pub struct PatternRouter;

impl MessageRouter for PatternRouter {
    fn resolve(&self, _from: &str, to: &str, agents: &HashSet<String>) -> Vec<String> {
        let pattern = match glob::Pattern::new(to) {
            Ok(p) => p,
            Err(_) => return vec![],
        };
        agents
            .iter()
            .filter(|id| pattern.matches(id))
            .cloned()
            .collect()
    }

    fn name(&self) -> &'static str {
        "pattern"
    }
}
