use std::collections::HashSet;

use super::MessageRouter;

/// Route messages by exact agent ID match.
///
/// This is the default topology and is fully backward-compatible with
/// Phase 2's hardcoded routing logic: `payload["to"]` must be the exact
/// ID of a registered agent.
#[derive(Debug, Clone, Copy)]
pub struct DirectRouter;

impl MessageRouter for DirectRouter {
    fn resolve(&self, _from: &str, to: &str, agents: &HashSet<String>) -> Vec<String> {
        if agents.contains(to) {
            vec![to.to_string()]
        } else {
            vec![]
        }
    }

    fn name(&self) -> &'static str {
        "direct"
    }
}
