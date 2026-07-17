use std::collections::HashSet;

use super::MessageRouter;

/// All traffic from non-hub agents routes through the designated hub.
///
/// * If the sender (`from`) **is** the hub → route directly to the target
///   (hub is forwarding a message).
/// * If the sender is **not** the hub → route to the hub (the message
///   will be processed by the hub agent, which may then forward it).
///
/// The hub agent is a normal WASM agent that happens to be designated as
/// the routing hub.  It receives all peer messages from spokes, reads
/// them via `cage_inbox_read`, and can forward them by calling
/// `cage_peer_send` with a different target (the hub becomes `from` and
/// the router then resolves directly).
#[derive(Debug, Clone)]
pub struct HubAndSpokeRouter {
    /// Agent ID of the hub.
    pub hub: String,
}

impl MessageRouter for HubAndSpokeRouter {
    fn resolve(&self, from: &str, to: &str, agents: &HashSet<String>) -> Vec<String> {
        if from == self.hub {
            // Hub is forwarding — deliver to the actual target
            if agents.contains(to) {
                vec![to.to_string()]
            } else {
                vec![]
            }
        } else {
            // Non-hub sender — route to the hub
            if agents.contains(&self.hub) {
                vec![self.hub.clone()]
            } else {
                vec![]
            }
        }
    }

    fn name(&self) -> &'static str {
        "hub-and-spoke"
    }
}
