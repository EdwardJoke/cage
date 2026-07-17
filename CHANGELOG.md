# Changelog

## [0.1.0] — 2026-07-17

### Added

- Single-agent WASM sandbox with fuel-limited execution via Wasmtime
- 9 host functions: log, send, time, random, env, HTTP, peer_send, inbox_pending, inbox_read
- Multi-agent orchestrator with spawn/kill/pause/resume lifecycle management
- 4 MessageRouter topologies: Direct, Broadcast, Pattern (glob), Hub-and-Spoke
- Dead Letter Queue (DLQ) for unroutable messages
- CLI subcommands: `run` (single agent) and `orchestrate` (multi-agent)
- Python bindings via PyO3 with full orchestrator API
- Example agents: agent-p0 (host function demo), agent-p1 (leader-worker orchestration)
- Environment variable injection for secrets and configuration

### Changed

- N/A

### Deprecated

- N/A

### Removed

- N/A

### Fixed

- N/A

### Security

- URL whitelisting for HTTP requests — agents can only call APIs on pre-approved hostnames
