# Cage — Agent Context

## Identity
Deterministic WASM sandbox for multi-agent orchestration. Single crate `coplex-cage`.

## Build & Run

```sh
cargo build                                  # debug build
cargo build --release                        # release build
cargo build -p agent-p0 --target wasm32-wasip1  # build example agent
cargo run -- run <agent.wasm>                # run single agent
cargo run -- orchestrate -a leader=agent.wasm -a worker-1=agent.wasm  # multi-agent
```

## Test

```sh
cargo test                                              # Rust tests
cd cage-py && pip install -e . && python -m pytest tests/  # Python tests (after building bindings)
```

## Project Specs (read before making decisions)

- `spec/commit.md` — Conventional Commits format
- `spec/branch.md` — Branch naming conventions
- `spec/version.md` — SemVer versioning rules
- `spec/changelog.md` — Keep a Changelog format

## Coding Conventions

- Use `anyhow::Result` for fallible functions, never panic in library code
- Log via the `log` crate (use `info!`/`debug!`/`warn!`), not `println`
- `SandboxState` holds per-agent runtime state; add new fields here rather than in `Sandbox`
- Router implementations go in `src/router/`; implement the `MessageRouter` trait
- WASM host functions register via `sandbox::register_host_functions`
- Keep `unsafe` out of the host; Wasmtime's `func_wrap` is safe

## Key Modules

| Module | File | Purpose |
|--------|------|---------|
| `main.rs` | `src/main.rs` | CLI with `run` and `orchestrate` subcommands |
| `lib.rs` | `src/lib.rs` | Re-exports all public API |
| `sandbox` | `src/sandbox.rs` | Single-agent Wasmtime wrapper, 9 host functions |
| `orchestrator` | `src/orchestrator.rs` | Multi-agent lifecycle, tick cycle, message routing |
| `router` | `src/router/` | 4 routing topologies + `MessageRouter` trait |
| `ipc` | `src/ipc.rs` | `HostMessage` and `AgentMessage` types |
| cage-py | `cage-py/src/lib.rs` | PyO3 bindings exposing Orchestrator/RoundSummary |

## Important Constraints

- Agents compile to `wasm32-wasip1` target
- Fuel limits execution — set via `store.set_fuel(n)`
- HTTP requests require URL whitelist (`allow_url`) — enforced in `is_url_allowed`
- Agent exports: `memory`, `_cage_alloc`, `_cage_free`, optional `_cage_init`/`_cage_tick`
- Max 64 agents per orchestrator (configurable)
