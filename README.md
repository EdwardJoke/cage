# Cage — Multi-Agent WASM Sandbox

Deterministic WebAssembly sandbox for orchestrating multi-agent systems with configurable fuel limits and pluggable message routing.

```
┌──────────────┐    peer_send     ┌──────────────┐
│   Agent A    │ ──────────────►  │    Agent B   │
│   (WASM)     │   inbox/outbox   │    (WASM)    │
└──────┬───────┘                  └──────┬───────┘
       │                                 │
       └───────────── host ──────────────┘
```

## Features

- **Deterministic execution** — instruction-level fuel budgets via Wasmtime
- **Inter-agent messaging** — 4 routing topologies (direct, broadcast, pattern, hub-and-spoke)
- **9 host functions** — log, send, time, random, env, HTTP, peer messaging, inbox
- **Lifecycle management** — spawn, kill, pause, resume agents at runtime
- **CLI + Python bindings** — drive orchestration from shell or scripts
- **Dead Letter Queue** — observe and drain unroutable messages

## Quick Start

```bash
# Install the CLI from crates.io
cargo install coplex-cage

# Install Python bindings from PyPI
pip install coplex-cage

# Build example agents (requires wasm32-wasip1 target)
rustup target add wasm32-wasip1
cargo build -p agent-p1 --target wasm32-wasip1 --release

# Run multi-agent orchestration
cage orchestrate \
  --agent 'leader=target/wasm32-wasip1/release/agent_p1.wasm' \
  --agent 'worker-a=target/wasm32-wasip1/release/agent_p1.wasm' \
  --agent 'worker-b=target/wasm32-wasip1/release/agent_p1.wasm' \
  --message '{"role":"leader","tasks":[{"data":"t1"},{"data":"t2"},{"data":"t3"}]}' \
  --rounds 6 --fuel 3000000 -v
```

See [docs/quickstart.md](docs/quickstart.md) for the full tutorial.

> **Install from source?** Clone the repo and run `cargo build --release` instead of `cargo install`.

## Documentation

- [Installation](docs/install.md) — build requirements and setup
- [Quick Start](docs/quickstart.md) — your first multi-agent run
- [Architecture](docs/architecture.md) — tick cycle and message flow
- [Host Functions](docs/host-functions.md) — WASM ABI reference
- [Message Routing](docs/routing.md) — topologies and DLQ
- [Python Bindings](docs/python.md) — PyO3 API and examples
- [Agent Development](docs/agent-development.md) — write your own agent
- [CLI Reference](docs/cli.md) — command-line flags

## License

Apache 2.0 — © Coplex
