# Cage

Deterministic WASM sandbox for multi-agent orchestration. Agents are compiled to `wasm32-wasip1`, run inside Wasmtime with fuel-limited execution, and communicate via a pluggable message router with 4 topologies.

## Repository

- **Source:** https://codeberg.org/EdwardJoke/cage
- **License:** Apache-2.0
- **Package:** `coplex-cage` (crates.io) / `coplex-cage` (PyPI)

## Directory Layout

```
cage/
├── Cargo.toml              # Workspace root; members: examples/agent-p0, agent-p1, cage-py
├── src/
│   ├── main.rs             # CLI binary: "cage run" (single agent) and "cage orchestrate" (multi-agent)
│   ├── lib.rs              # Library root; re-exports ipc, orchestrator, router, sandbox
│   ├── ipc.rs              # HostMessage / AgentMessage structs (JSON-serializable)
│   ├── sandbox.rs          # Single-agent Wasmtime wrapper: load, init, tick, fuel metering
│   ├── orchestrator.rs     # Multi-agent lifecycle: spawn, tick_all, pause, resume, kill, route, DLQ
│   └── router/
│       ├── mod.rs          # Topology enum, RouterConfig, MessageRouter trait
│       ├── direct.rs       # DirectRouter — exact-match routing
│       ├── broadcast.rs    # BroadcastRouter — send to all but sender
│       ├── pattern.rs      # PatternRouter — glob/wildcard routing
│       └── hub_and_spoke.rs# HubAndSpokeRouter — hub-mediated routing
├── docs/                   # User-facing documentation: install, quickstart, architecture, CLI, host functions, routing, Python, agent development
├── examples/
│   ├── agent-p0/           # WASM agent demoing all 9 host functions
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── agent-p1/           # Leader-worker task distribution WASM agent
│       ├── Cargo.toml
│       └── src/lib.rs
├── cage-py/                # Python bindings (PyO3 + maturin)
│   ├── Cargo.toml
│   ├── pyproject.toml
│   ├── src/lib.rs          # PyO3 classes: Orchestrator, RoundSummary
│   ├── python/cage/
│   │   ├── __init__.py     # High-level Python API (Policy, Sandbox)
│   │   └── types.py        # Type definitions (AgentMessage, SandboxStats)
│   └── tests/
│       ├── test_sandbox.py
│       ├── test_orchestrator.py
│       └── _run_integration.py
├── spec/                   # Project specifications
│   ├── changelog.md        # Keep a Changelog spec
│   ├── version.md          # SemVer 2.0.0 spec
│   ├── commit.md           # Conventional Commits spec
│   └── branch.md           # Branch naming spec
└── CHANGELOG.md            # Release changelog
```

## Tech Stack

- **Runtime:** Rust (edition 2024), Wasmtime 46.x, wasmtime-wasi (p1)
- **CLI:** clap (derive)
- **HTTP:** ureq (rustls)
- **Serialization:** serde + serde_json
- **Python bindings:** PyO3 + maturin

## Key Concepts

### Tick Cycle (3 phases)
1. **Deliver Inbox** — orchestrator writes pending messages into agent memory
2. **Execute Tick** — calls `_cage_tick()`, agent processes inbox, calls host functions
3. **Drain Outbox** — collects peer messages from agent outbox, routes via MessageRouter

### Agent Lifecycle
`spawn` → `init` → `tick` (repeated) → `pause` / `resume` → `kill`

### Host Functions (imported from `"cage"` module)
`log`, `send`, `time_now`, `random`, `env_get`, `http_request`, `peer_send`, `inbox_pending`, `inbox_read`

### Routing Topologies
- **Direct** — exact agent ID match via `payload["to"]`
- **Broadcast** — every agent except sender
- **Pattern** — glob patterns (`worker-*`, `logs-#`)
- **Hub-and-Spoke** — all traffic through a designated hub

### Dead Letter Queue (DLQ)
Captures unroutable messages for inspection instead of silently dropping them.

## WASM Agent Requirements

Export (from `wasm32-wasip1`):
- `memory` — linear memory
- `_cage_alloc(size: i32) -> i32`
- `_cage_free(ptr: i32)`
- `_cage_init(msg_ptr: i32, msg_len: i32) -> i32` (optional, skipped if absent)
- `_cage_tick() -> i32` (optional)
