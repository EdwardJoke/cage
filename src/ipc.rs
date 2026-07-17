use serde::{Deserialize, Serialize};
use serde_json::Value;

#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize)]
pub struct HostMessage {
    pub kind: String,
    pub payload: Value,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub kind: String,
    pub payload: Value,
}

#[allow(dead_code)]
impl HostMessage {
    pub fn new(kind: impl Into<String>, payload: Value) -> Self {
        Self {
            kind: kind.into(),
            payload,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("HostMessage serialization should not fail")
    }
}
