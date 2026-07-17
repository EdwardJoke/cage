// Cage — Agent WASM sandbox library
//
// Re-exports the sandbox and IPC modules so other crates (e.g. cage-py, agent-p0 tooling)
// can use the runtime without reimplementing the bridge.

pub mod ipc;
pub mod orchestrator;
pub mod router;
pub mod sandbox;

pub use sandbox::Sandbox;
